//! SSA aliases within a path: only loop-carried words need mutable storage.
use super::*;
#[derive(Clone)]
struct Frame {
    saved: Vec<(usize, Word)>,
}
#[derive(Clone)]
pub(super) struct Eval {
    pub env: Vec<Word>,
    frames: Vec<Frame>,
    pub guard: Bit,
    pub outcome: Option<(Word, Word)>,
}
fn integer(s: &str, width: usize) -> Word {
    KnownBits::integer(s, width)
        .0
        .into_iter()
        .map(|b| Bit::C(b.expect("literal bit")))
        .collect()
}
impl Eval {
    pub fn new(plan: &Plan, source: &source::Source) -> Self {
        let mutable: BTreeSet<_> = source
            .rows
            .iter()
            .flat_map(|i| i.locals.iter().copied())
            .collect();
        let root = plan.node(0, &[]).expect("entry");
        let env = plan
            .locations()
            .iter()
            .enumerate()
            .map(|(id, l)| {
                if let Some(input) = source.inputs.get(&id) {
                    input.clone()
                } else if mutable.contains(&id) {
                    constant(0, l.width)
                } else {
                    root.values[id]
                        .0
                        .iter()
                        .map(|b| Bit::C(b.expect("bound classical argument")))
                        .collect()
                }
            })
            .collect();
        Self {
            env,
            frames: vec![],
            guard: Bit::C(true),
            outcome: None,
        }
    }
    pub fn read(&self, source: &source::Source, r: &Read) -> Word {
        match r {
            Read::Location(id) => self.env[*id].clone(),
            Read::Integer(s) => integer(s, source.ptr_width),
            _ => unreachable!("proved value operand"),
        }
    }
    pub fn path(&mut self, source: &source::Source, path: &[proof::Step], b: &mut Build) {
        for step in path {
            self.row(source, step, b);
        }
    }
    fn row(&mut self, source: &source::Source, step: &proof::Step, b: &mut Build) {
        let i = &source.rows[step.key.pc];
        match i.op.as_str() {
            "qjump" | "qjumpr" => (),
            "qbranch" => {
                if let Some(value) = step.branch {
                    let c = self.read(source, &i.operands[0])[0].clone();
                    self.guard = b.and(&self.guard, &if value { c } else { not(&c) });
                }
            }
            "call" => {
                let Read::Symbol(name) = &i.operands[0] else {
                    unreachable!()
                };
                let Read::List(ids) = &i.operands[1] else {
                    unreachable!()
                };
                let actuals: Vec<_> = ids.iter().map(|id| self.env[*id].clone()).collect();
                let locals = &source.rows[source.entries[name]].locals;
                let saved = locals
                    .iter()
                    .map(|id| (*id, self.env[*id].clone()))
                    .collect();
                for id in locals.iter() {
                    self.env[*id] = constant(0, self.env[*id].len());
                }
                for (id, value) in source.parameters[name].iter().zip(actuals) {
                    self.env[*id] = value;
                }
                self.frames.push(Frame { saved });
            }
            "ret" => {
                let value = self.read(source, &i.operands[0]);
                let frame = self.frames.pop().expect("proved static return");
                for (id, value) in frame.saved {
                    self.env[id] = value;
                }
                let site = step.key.calls.last().expect("proved caller");
                self.env[source.rows[*site].lhs[0]] = value;
            }
            "retval" => {
                self.outcome = Some((
                    self.read(source, &i.operands[0]),
                    constant(0, source.fault.len()),
                ))
            }
            "fault" => {
                let Read::Integer(code) = &i.operands[0] else {
                    unreachable!()
                };
                self.outcome = Some((
                    constant(0, source.target.len()),
                    integer(code, source.fault.len()),
                ));
            }
            "parmov" => {
                let Read::List(ids) = &i.operands[0] else {
                    unreachable!()
                };
                let values: Vec<_> = ids.iter().map(|id| self.env[*id].clone()).collect();
                for (id, value) in i.lhs.iter().zip(values) {
                    self.env[*id] = value;
                }
            }
            _ => {
                let width = self.env[i.lhs[0]].len();
                let value = if i.op.starts_with("const.i") {
                    let Read::Integer(s) = &i.operands[0] else {
                        unreachable!()
                    };
                    integer(s, width)
                } else if i.op == "label" {
                    let Read::Label(pc) = i.operands[0] else {
                        unreachable!()
                    };
                    constant(pc as u128, width)
                } else {
                    self.operation(source, i, width, b)
                };
                self.env[i.lhs[0]] = value;
            }
        }
    }
    fn operation(
        &self,
        source: &source::Source,
        i: &source::Instruction,
        width: usize,
        b: &mut Build,
    ) -> Word {
        let a = self.read(source, &i.operands[0]);
        if i.op == "mov"
            || i.op.starts_with("zext.")
            || i.op.starts_with("sext.")
            || i.op.starts_with("trunc.")
        {
            return resize(&a, width, i.op.starts_with("sext."));
        }
        let c = self.read(source, &i.operands[1]);
        match i.op.as_str() {
            "select" => b.mux_word(&a[0], &c, &self.read(source, &i.operands[2])),
            "add" | "sub" => b.add(&a, &c, i.op == "sub"),
            "mul" => b.multiply(&a, &c),
            "and" => a.iter().zip(&c).map(|(a, c)| b.and(a, c)).collect(),
            "or" => a.iter().zip(&c).map(|(a, c)| b.or(a, c)).collect(),
            "xor" => a.iter().zip(&c).map(|(a, c)| b.xor(a, c)).collect(),
            "shl" | "lshr" | "ashr" => b.shift(&a, &c, &i.op),
            _ => vec![super::super::machine::rows::compare(
                b,
                &a,
                &c,
                i.op.strip_prefix("icmp.").expect("proved kernel"),
            )],
        }
    }
}
