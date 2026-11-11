use super::*;
impl Recipes {
    pub(super) fn summarize(&mut self) -> Result<()> {
        let (prefix, cycle) = self.plan.layers().descriptors();
        for step in 0..(prefix.len() + cycle.len()) {
            let built =
                self.machine
                    .layer(&self.plan, &self.source, step as u64, self.machine.fixed);
            let layout = layout(&built, self.machine.fixed);
            if step < prefix.len() {
                self.prefix.push(layout);
            } else {
                self.cycle.push(layout);
            }
        }
        self.commit = layout(
            &self.machine.commit(&self.source, self.machine.fixed),
            self.machine.fixed,
        );
        self.counts = self.measure();
        Ok(())
    }
    pub(super) fn layout(&self, step: u64) -> &Layout {
        if step < self.prefix.len() as u64 {
            &self.prefix[step as usize]
        } else {
            &self.cycle[((step - self.prefix.len() as u64) % self.cycle.len() as u64) as usize]
        }
    }
    fn allocated_before(&self, step: u64) -> u128 {
        self.weighted(step, |layout| layout.alloc as u128)
    }
    fn weighted(&self, steps: u64, value: impl Fn(&Layout) -> u128) -> u128 {
        let prefix = steps.min(self.prefix.len() as u64) as usize;
        let rest = steps - prefix as u64;
        self.prefix[..prefix].iter().map(&value).sum::<u128>()
            + u128::from(rest / self.cycle.len() as u64)
                * self.cycle.iter().map(&value).sum::<u128>()
            + self.cycle[..(rest % self.cycle.len() as u64) as usize]
                .iter()
                .map(value)
                .sum::<u128>()
    }
    fn measure(&self) -> Counts {
        let steps = self.plan.binding().budget.steps;
        let forward = self.weighted(steps, |l| l.gates as u128);
        let history_wires = self.allocated_before(steps);
        let mut histogram = BTreeMap::new();
        let controls = self
            .prefix
            .iter()
            .chain(&self.cycle)
            .flat_map(|l| l.hist.keys())
            .chain(self.commit.hist.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        for arity in controls {
            histogram.insert(
                arity,
                2 * self.weighted(steps, |l| l.hist.get(&arity).copied().unwrap_or(0))
                    + self.commit.hist.get(&arity).copied().unwrap_or(0),
            );
        }
        let mut kernels = Kernels::default();
        kernels.constant_multiplications =
            self.weighted(steps, |l| l.kernels.constant_multiplications);
        kernels.generic_multiplications =
            self.weighted(steps, |l| l.kernels.generic_multiplications);
        kernels.fixed_shifts = self.weighted(steps, |l| l.kernels.fixed_shifts);
        kernels.variable_shifts = self.weighted(steps, |l| l.kernels.variable_shifts);
        kernels.equalities = self.weighted(steps, |l| l.kernels.equalities);
        kernels.ordered_comparisons = self.weighted(steps, |l| l.kernels.ordered_comparisons);
        Counts {
            forward,
            commit: self.commit.gates as u128,
            full: 2 * forward + self.commit.gates as u128,
            max_controls: histogram
                .iter()
                .filter(|(_, count)| **count > 0)
                .map(|(arity, _)| *arity)
                .max()
                .unwrap_or(0),
            histogram,
            qubits: self.machine.fixed as u128 + history_wires + self.commit.alloc as u128,
            history_wires,
            peak_gate_objects: self
                .prefix
                .iter()
                .chain(&self.cycle)
                .chain([&self.commit])
                .map(|l| l.gates)
                .max()
                .unwrap_or(0),
            layer_descriptors: self.prefix.len() + self.cycle.len(),
            source_row_contexts: self
                .plan
                .layers()
                .expanded_row_contexts(steps, self.plan.rows().len()),
            kernels,
        }
    }
    /// Emit actual numeric X/control operations, forward; commit; inverse.
    /// Visitor errors are propagated immediately; no truncated success receipt.
    pub fn visit(&self, mut visitor: impl FnMut(&Flip) -> Result<()>) -> Result<()> {
        self.wire_count()?;
        let steps = self.plan.binding().budget.steps;
        for step in 0..steps {
            self.visit_layer(step, false, &mut visitor)?;
        }
        let offset = self.machine.fixed as u128 + self.allocated_before(steps);
        let built = self.machine.commit(
            &self.source,
            usize::try_from(offset).map_err(|_| Error("host limit: recipe offset".into()))?,
        );
        assert_eq!(
            built.gates.len(),
            self.commit.gates,
            "commit recipe count changed"
        );
        for gate in &built.gates {
            visitor(gate)?;
        }
        for step in (0..steps).rev() {
            self.visit_layer(step, true, &mut visitor)?;
        }
        Ok(())
    }
    fn visit_layer(
        &self,
        step: u64,
        inverse: bool,
        visitor: &mut impl FnMut(&Flip) -> Result<()>,
    ) -> Result<()> {
        let offset = self.machine.fixed as u128 + self.allocated_before(step);
        let start =
            usize::try_from(offset).map_err(|_| Error("host limit: recipe offset".into()))?;
        let built = self.machine.layer(&self.plan, &self.source, step, start);
        let expected = self.layout(step);
        assert_eq!(
            built.next - start,
            expected.alloc,
            "allocation recipe changed"
        );
        assert_eq!(built.gates.len(), expected.gates, "gate recipe changed");
        if inverse {
            for gate in built.gates.iter().rev() {
                visitor(gate)?;
            }
        } else {
            for gate in &built.gates {
                visitor(gate)?;
            }
        }
        Ok(())
    }
    /// Compatibility adapter; retains all IR nodes. Streaming clients use visit.
    pub fn to_program(&self) -> Result<Program> {
        let mut program = self.program_layout()?;
        self.visit(|gate| {
            program.circuits[0].body.push(Node::Gate(GateInst {
                gate: BaseGate::X,
                own_controls: gate.controls.len(),
                controls: gate
                    .controls
                    .iter()
                    .map(|(q, p)| Ctrl {
                        qubit: self.qubit(*q),
                        positive: *p,
                    })
                    .collect(),
                targets: vec![self.qubit(gate.target)],
            }));
            Ok(())
        })?;
        Ok(program)
    }
    /// Geometry/roles/history provenance with an empty body, for stream headers.
    pub fn program_layout(&self) -> Result<Program> {
        let private = self.wire_count()? - self.interface_wires();
        let mut workspace = vec![];
        let fixed = self.machine.fixed - self.interface_wires();
        let history = usize::try_from(self.counts.history_wires)
            .map_err(|_| Error("host limit: history wires".into()))?;
        for (name, count, kind) in [
            ("state", fixed, WsKind::Clean),
            ("history", history, WsKind::RetainedHistory),
            ("commit", private - fixed - history, WsKind::Clean),
        ] {
            for chunk in 0..count.div_ceil(u32::MAX as usize) {
                workspace.push(Workspace {
                    name: format!("${name}{chunk}"),
                    width: (count - chunk * u32::MAX as usize).min(u32::MAX as usize) as u32,
                    kind,
                    class: "any".into(),
                });
            }
        }
        let circuit = Circuit {
            name: self.source.name.clone(),
            perm: true,
            mut_bundles: self
                .source
                .bundles
                .iter()
                .filter(|b| b.role == Role::Mut)
                .cloned()
                .collect(),
            in_bundles: self
                .source
                .bundles
                .iter()
                .filter(|b| b.role == Role::In)
                .cloned()
                .collect(),
            mem: vec![],
            workspace,
            local_regions: vec![],
            body: vec![],
        };
        Ok(Program {
            top: self.source.name.clone(),
            circuits: vec![circuit],
        })
    }
    fn qubit(&self, wire: usize) -> QubitRef {
        if wire < self.interface_wires() {
            return self.source.interface[wire].clone();
        }
        let end_history = self.machine.fixed + self.counts.history_wires as usize;
        let (name, index) = if wire < self.machine.fixed {
            ("state", wire - self.interface_wires())
        } else if wire < end_history {
            ("history", wire - self.machine.fixed)
        } else {
            ("commit", wire - end_history)
        };
        QubitRef::new(
            &format!("${name}{}", index / u32::MAX as usize),
            (index % u32::MAX as usize) as u32,
        )
    }
}
fn layout(b: &Build, start: usize) -> Layout {
    let mut hist = BTreeMap::new();
    for gate in &b.gates {
        *hist.entry(gate.controls.len()).or_default() += 1;
    }
    Layout {
        alloc: b.next - start,
        gates: b.gates.len(),
        hist,
        kernels: b.kernels.clone(),
    }
}
