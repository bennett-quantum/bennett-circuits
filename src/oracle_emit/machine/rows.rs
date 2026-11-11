use super::*;
impl Machine {
    pub fn row(
        &self,
        plan: &Plan,
        source: &Source,
        node: &PlannedNode,
        at: &Bit,
        next: &Word,
        b: &mut Build,
    ) {
        let i = &source.rows[node.key.pc];
        match i.op.as_str() {
            "call" => self.call(plan, source, node, at, next, b),
            "ret" => self.ret(source, node, at, next, b),
            "retval" => {
                let value = self.read(source, node, &i.operands[0]);
                b.copy(&value, &self.result, &[at.clone()]);
                self.halt_with(0, &[at.clone()], next, b);
            }
            "qbranch" => {
                let condition = self.read(source, node, &i.operands[0])[0].clone();
                for (operand, predicate) in [
                    (&i.operands[1], condition.clone()),
                    (&i.operands[2], not(&condition)),
                ] {
                    let Read::Label(pc) = operand else {
                        unreachable!()
                    };
                    self.go(
                        RowKey {
                            pc: *pc,
                            calls: node.key.calls.clone(),
                        },
                        &[at.clone(), predicate],
                        next,
                        b,
                    );
                }
            }
            "fault" | "faultif" => self.fault(source, node, at, next, b),
            "qjump" | "qjumpr" => {
                if let Some(target) = node.successors.iter().find_map(|e| e.target.clone()) {
                    self.go(target, &[at.clone()], next, b);
                } else {
                    self.halt_with(4, &[at.clone()], next, b);
                }
            }
            _ => {
                let values = self.values(source, node, i, b);
                self.store(node, values, at, b);
                self.sequential(node, at, next, b);
            }
        }
    }
    fn sequential(&self, node: &PlannedNode, at: &Bit, next: &Word, b: &mut Build) {
        let target = node
            .successors
            .iter()
            .find_map(|e| e.target.clone())
            .expect("validated sequential edge");
        self.go(target, &[at.clone()], next, b);
    }
    fn call(
        &self,
        plan: &Plan,
        source: &Source,
        node: &PlannedNode,
        at: &Bit,
        next: &Word,
        b: &mut Build,
    ) {
        let Event::Call { cells, check, .. } = node.event else {
            unreachable!()
        };
        if check == Check::Fault {
            self.halt_with(3, &[at.clone()], next, b);
            return;
        }
        let sum = if self.stack.is_empty() {
            vec![]
        } else {
            b.add(
                &resize(&self.stack, self.stack.len() + 1, false),
                &constant(u128::from(cells), self.stack.len() + 1),
                false,
            )
        };
        let bad = if check == Check::Dynamic {
            b.ult(
                &constant(u128::from(plan.binding().budget.stack), sum.len()),
                &sum,
            )
        } else {
            Bit::C(false)
        };
        self.halt_with(3, &[at.clone(), bad.clone()], next, b);
        let active = b.and(at, &not(&bad));
        let i = &source.rows[node.key.pc];
        let Read::Symbol(name) = &i.operands[0] else {
            unreachable!()
        };
        let Read::List(actuals) = &i.operands[1] else {
            unreachable!()
        };
        let mut calls = node.key.calls.clone();
        calls.push(node.key.pc);
        for (id, argument) in source.parameters[name].iter().zip(actuals) {
            let value = self.read_location(source, node, *argument);
            for (bit, wire) in self.banks[&(calls.clone(), *id)].iter().enumerate() {
                if let Some(q) = wire {
                    b.copy(
                        &vec![value[bit].clone()],
                        &vec![Bit::Q(*q, true)],
                        &[active.clone()],
                    );
                }
            }
        }
        if !self.stack.is_empty() {
            let history = b.alloc(self.stack.len());
            b.copy(
                &resize(&sum, self.stack.len(), false),
                &history,
                &[active.clone()],
            );
            for (a, c) in self.stack.iter().zip(&history) {
                b.exchange(a, c, &[active.clone()]);
            }
        }
        self.go(
            RowKey {
                pc: source.entries[name],
                calls,
            },
            &[active],
            next,
            b,
        );
    }
    fn ret(&self, source: &Source, node: &PlannedNode, at: &Bit, next: &Word, b: &mut Build) {
        let mut parent = node.key.calls.clone();
        let Some(site) = parent.pop() else {
            self.halt_with(4, &[at.clone()], next, b);
            return;
        };
        let value = self.read(source, node, &source.rows[node.key.pc].operands[0]);
        let caller = source.rows[site].lhs[0];
        // Delivery precedes clearing; its captured RHS cannot be lost with the callee.
        let mut parent_node = node.clone();
        parent_node.key.calls = parent.clone();
        self.store(&parent_node, vec![(caller, value)], at, b);
        for id in source.rows[node.key.pc].locals.iter() {
            for (bit, wire) in self.banks[&(node.key.calls.clone(), *id)]
                .iter()
                .enumerate()
            {
                let Some(q) = wire else {
                    continue;
                };
                let target = Bit::Q(*q, true);
                match node.values[*id].0[bit] {
                    Some(false) => (),
                    Some(true) => b.flip(&target, &[at.clone()]),
                    None => {
                        let history = b.alloc(1)[0].clone();
                        b.exchange(&target, &history, &[at.clone()]);
                    }
                }
            }
        }
        self.go(
            RowKey {
                pc: site + 1,
                calls: parent,
            },
            &[at.clone()],
            next,
            b,
        );
    }
    fn fault(&self, source: &Source, node: &PlannedNode, at: &Bit, next: &Word, b: &mut Build) {
        let i = &source.rows[node.key.pc];
        let conditional = i.op == "faultif";
        let bad = if conditional {
            self.read(source, node, &i.operands[0])[0].clone()
        } else {
            Bit::C(true)
        };
        let Read::Integer(code) = &i.operands[usize::from(conditional)] else {
            unreachable!()
        };
        let code = KnownBits::integer(code, source.fault.len())
            .as_u128()
            .unwrap() as u8;
        self.halt_with(code, &[at.clone(), bad.clone()], next, b);
        if bad != Bit::C(true) {
            self.go(
                RowKey {
                    pc: node.key.pc + 1,
                    calls: node.key.calls.clone(),
                },
                &[at.clone(), not(&bad)],
                next,
                b,
            );
        }
    }
    fn values(
        &self,
        source: &Source,
        node: &PlannedNode,
        i: &Instruction,
        b: &mut Build,
    ) -> Vec<(usize, Word)> {
        if i.op == "parmov" {
            let Read::List(ids) = &i.operands[0] else {
                unreachable!()
            };
            return i
                .lhs
                .iter()
                .zip(ids)
                .map(|(dst, src)| (*dst, self.read_location(source, node, *src)))
                .collect();
        }
        let width = node.values[i.lhs[0]].0.len();
        let value = if i.op.starts_with("const.i") {
            let Read::Integer(s) = &i.operands[0] else {
                unreachable!()
            };
            KnownBits::integer(s, width)
                .0
                .into_iter()
                .map(|v| Bit::C(v.unwrap()))
                .collect()
        } else if i.op == "label" {
            let Read::Label(pc) = i.operands[0] else {
                unreachable!()
            };
            constant(pc as u128, width)
        } else {
            self.operation(source, node, i, width, b)
        };
        vec![(i.lhs[0], value)]
    }
    fn operation(
        &self,
        source: &Source,
        node: &PlannedNode,
        i: &Instruction,
        width: usize,
        b: &mut Build,
    ) -> Word {
        let a = self.read(source, node, &i.operands[0]);
        if i.op == "mov" {
            return resize(&a, width, false);
        }
        if i.op.starts_with("zext.") || i.op.starts_with("sext.") || i.op.starts_with("trunc.") {
            return resize(&a, width, i.op.starts_with("sext"));
        }
        let second = self.read(source, node, &i.operands[1]);
        match i.op.as_str() {
            "select" => b.mux_word(&a[0], &second, &self.read(source, node, &i.operands[2])),
            "add" | "sub" => b.add(&a, &second, i.op == "sub"),
            "mul" => b.multiply(&a, &second),
            "and" => a.iter().zip(&second).map(|(a, c)| b.and(a, c)).collect(),
            "or" => a.iter().zip(&second).map(|(a, c)| b.or(a, c)).collect(),
            "xor" => a.iter().zip(&second).map(|(a, c)| b.xor(a, c)).collect(),
            "shl" | "lshr" | "ashr" => b.shift(&a, &second, &i.op),
            _ => vec![compare(
                b,
                &a,
                &second,
                i.op.strip_prefix("icmp.").expect("validated kernel"),
            )],
        }
    }
}
fn compare(b: &mut Build, a: &Word, c: &Word, predicate: &str) -> Bit {
    if predicate == "eq" {
        return b.eq(a, c);
    }
    if predicate == "ne" {
        return not(&b.eq(a, c));
    }
    let mut less = b.ult(a, c);
    if predicate.starts_with('s') {
        let signs = b.xor(a.last().unwrap(), c.last().unwrap());
        less = b.mux(&signs, a.last().unwrap(), &less);
    }
    if predicate.ends_with("le") {
        let equal = b.eq(a, c);
        b.or(&less, &equal)
    } else {
        less
    }
}
