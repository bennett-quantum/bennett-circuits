use super::*;
use crate::oracle_plan::{Check, Event, Node as PlannedNode};
use source::{Instruction, Read, Source};
mod rows;
pub(super) struct Machine {
    pub banks: BTreeMap<(Vec<usize>, usize), Vec<Option<usize>>>,
    pub tags: BTreeMap<RowKey, usize>,
    pub pc: Word,
    pub code: Word,
    pub result: Word,
    pub stack: Word,
    pub fixed: usize,
    pub halt: usize,
}
impl Machine {
    pub fn new(plan: &Plan, source: &Source) -> Self {
        let mut builder = Build::new(source.interface.len());
        let tags = plan
            .nodes()
            .keys()
            .enumerate()
            .map(|(i, k)| (k.clone(), i))
            .collect::<BTreeMap<_, _>>();
        let halt = tags.len();
        let width = (usize::BITS - halt.leading_zeros()).max(1) as usize;
        let pc = builder.alloc(width);
        let code = builder.alloc(source.fault.len());
        let result = builder.alloc(source.target.len());
        let dynamic = plan.nodes().values().any(|n| {
            matches!(
                n.event,
                Event::Call {
                    check: Check::Dynamic,
                    ..
                }
            )
        });
        let stack = if dynamic {
            builder.alloc((u64::BITS - plan.binding().budget.stack.leading_zeros()).max(1) as usize)
        } else {
            vec![]
        };
        let banks = banks(plan, source, &mut builder);
        Self {
            banks,
            tags,
            pc,
            code,
            result,
            stack,
            fixed: builder.next,
            halt,
        }
    }
    pub fn read_location(&self, source: &Source, node: &PlannedNode, id: usize) -> Word {
        node.values[id]
            .0
            .iter()
            .enumerate()
            .map(|(bit, known)| {
                if let Some(v) = known {
                    return Bit::C(*v);
                }
                if let Some(word) = source.inputs.get(&id) {
                    return word[bit].clone();
                }
                let wire = self.banks[&(node.key.calls.clone(), id)][bit]
                    .expect("live unknown bit must have storage");
                Bit::Q(wire, true)
            })
            .collect()
    }
    pub fn read(&self, source: &Source, node: &PlannedNode, operand: &Read) -> Word {
        match operand {
            Read::Location(id) => self.read_location(source, node, *id),
            Read::Integer(s) => KnownBits::integer(s, source.ptr_width)
                .0
                .into_iter()
                .map(|v| Bit::C(v.unwrap()))
                .collect(),
            _ => panic!("validated recipe value operand"),
        }
    }
    pub fn layer(&self, plan: &Plan, source: &Source, step: u64, start: usize) -> Build {
        let mut b = Build::new(start);
        let mut dispatch = vec![];
        for key in plan.layer(step) {
            let id = if key.pc == plan.rows().len() {
                self.halt
            } else {
                self.tags[key]
            };
            let at = b.equal_flag(&self.pc, id);
            dispatch.push((key, at));
        }
        let next = b.alloc(self.pc.len());
        for (key, at) in dispatch {
            if key.pc == plan.rows().len() {
                b.copy(&constant(self.halt as u128, next.len()), &next, &[at]);
            } else {
                self.row(plan, source, &plan.nodes()[key], &at, &next, &mut b);
            }
        }
        for (old, new) in self.pc.iter().zip(&next) {
            b.exchange(old, new, &[]);
        }
        b
    }
    pub fn commit(&self, source: &Source, start: usize) -> Build {
        let mut b = Build::new(start);
        b.copy(
            &self.result,
            &source.target.clone().map(|q| Bit::Q(q, true)).collect(),
            &[],
        );
        b.copy(
            &self.code,
            &source.fault.clone().map(|q| Bit::Q(q, true)).collect(),
            &[],
        );
        let halt = b.equal_flag(&self.pc, self.halt);
        b.copy(
            &constant(1, source.fault.len()),
            &source.fault.clone().map(|q| Bit::Q(q, true)).collect(),
            &[not(&halt)],
        );
        let controls = self
            .pc
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if self.halt >> i & 1 != 0 {
                    p.clone()
                } else {
                    not(p)
                }
            })
            .collect::<Vec<_>>();
        b.flip(&halt, &controls);
        b
    }
    fn go(&self, key: RowKey, at: &[Bit], next: &Word, b: &mut Build) {
        if at.contains(&Bit::C(false)) {
            return;
        }
        let id = self.tags[&key];
        b.copy(&constant(id as u128, next.len()), next, at);
    }
    fn halt_with(&self, code: u8, at: &[Bit], next: &Word, b: &mut Build) {
        b.copy(&constant(u128::from(code), self.code.len()), &self.code, at);
        b.copy(&constant(self.halt as u128, next.len()), next, at);
    }
    fn store(&self, node: &PlannedNode, values: Vec<(usize, Word)>, at: &Bit, b: &mut Build) {
        // Capture every RHS before any destination changes (parallel aliases).
        let captured = values
            .into_iter()
            .map(|(id, value)| {
                let mut capture = value.clone();
                for (i, bit) in value.iter().enumerate() {
                    if matches!(bit, Bit::Q(..))
                        && self.banks[&(node.key.calls.clone(), id)][i].is_some()
                    {
                        capture[i] = b.alloc(1)[0].clone();
                        b.copy(&vec![bit.clone()], &vec![capture[i].clone()], &[at.clone()]);
                    }
                }
                (id, capture)
            })
            .collect::<Vec<_>>();
        for (id, value) in captured {
            for (i, wire) in self.banks[&(node.key.calls.clone(), id)].iter().enumerate() {
                let Some(wire) = wire else {
                    continue;
                };
                let target = Bit::Q(*wire, true);
                match node.values[id].0[i] {
                    Some(true) => b.flip(&target, &[at.clone()]),
                    Some(false) => (),
                    None => {
                        let history = b.alloc(1)[0].clone();
                        b.exchange(&target, &history, &[at.clone()]);
                    }
                }
                b.copy(&vec![value[i].clone()], &vec![target], &[at.clone()]);
            }
        }
    }
}
fn banks(
    plan: &Plan,
    source: &Source,
    b: &mut Build,
) -> BTreeMap<(Vec<usize>, usize), Vec<Option<usize>>> {
    let mut live: BTreeMap<(Vec<usize>, usize), Vec<bool>> = BTreeMap::new();
    for node in plan.nodes().values() {
        let row = &source.rows[node.key.pc];
        for id in row.locals.iter() {
            live.entry((node.key.calls.clone(), *id))
                .or_insert(vec![false; plan.locations()[*id].width]);
        }
        for id in row.operands.iter().flat_map(Read::locations) {
            if source.inputs.contains_key(&id) {
                continue;
            }
            let mask =
                live.entry((node.key.calls.clone(), id))
                    .or_insert(vec![false; plan.locations()[id].width]);
            for (used, fact) in mask.iter_mut().zip(&node.values[id].0) {
                *used |= fact.is_none();
            }
        }
    }
    live.into_iter()
        .map(|(key, bits)| {
            (
                key,
                bits.into_iter()
                    .map(|used| {
                        if used {
                            let Bit::Q(q, _) = b.alloc(1)[0] else {
                                unreachable!()
                            };
                            Some(q)
                        } else {
                            None
                        }
                    })
                    .collect(),
            )
        })
        .collect()
}
