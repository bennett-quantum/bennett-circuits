use super::*;
use model::Model;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct State {
    pub values: Vec<KnownBits>,
    pub saved: Vec<Vec<KnownBits>>,
    pub stack: Interval,
}
impl State {
    fn join(&mut self, other: &Self, limit: u64) -> bool {
        let mut changed = false;
        for (a, b) in self.values.iter_mut().zip(&other.values) {
            changed |= a.join(b);
        }
        assert_eq!(self.saved.len(), other.saved.len());
        for (a, b) in self
            .saved
            .iter_mut()
            .flatten()
            .zip(other.saved.iter().flatten())
        {
            changed |= a.join(b);
        }
        let min = self.stack.min.min(other.stack.min);
        // Widen an increasing counter to its admitted upper bound. Return never
        // decrements sp, and a failing call has no running successor.
        let max = if self.stack.max != other.stack.max {
            limit
        } else {
            self.stack.max
        };
        changed |= self.stack != (Interval { min, max });
        self.stack = Interval { min, max };
        changed
    }
}
pub(super) fn solve(
    model: &Model,
    budget: Budget,
    order: AnalysisOrder,
) -> (BTreeMap<RowKey, Node>, Option<String>, usize) {
    if model.recursive {
        return general(model, budget, "recursive call context", 0);
    }
    let entry = RowKey {
        pc: 0,
        calls: vec![],
    };
    let mut states = BTreeMap::from([(
        entry.clone(),
        State {
            values: model.initial.clone(),
            saved: vec![],
            stack: Interval { min: 0, max: 0 },
        },
    )]);
    let mut queue = BTreeSet::from([entry]);
    let mut visits = 0;
    while let Some(key) = pop(&mut queue, model, order) {
        visits += 1;
        if states.len() > 4096 {
            return general(model, budget, "context descriptor widening", visits);
        }
        let transition = transfer::execute(model, &key, &states[&key], budget);
        for (next, state) in transition.states {
            let changed = if let Some(old) = states.get_mut(&next) {
                old.join(&state, budget.stack)
            } else {
                states.insert(next.clone(), state);
                true
            };
            if changed {
                queue.insert(next);
            }
        }
    }
    let nodes = states
        .iter()
        .map(|(key, state)| {
            (
                key.clone(),
                transfer::execute(model, key, state, budget).node,
            )
        })
        .collect();
    (nodes, None, visits)
}
fn pop(queue: &mut BTreeSet<RowKey>, model: &Model, order: AnalysisOrder) -> Option<RowKey> {
    let key = queue
        .iter()
        .min_by(|a, b| {
            let (ra, rb) = (&model.rows[a.pc].public, &model.rows[b.pc].public);
            match order {
                AnalysisOrder::Forward => a.cmp(b),
                AnalysisOrder::FunctionsReversed => rb.function.cmp(&ra.function).then(a.cmp(b)),
                AnalysisOrder::BlocksReversed => ra
                    .function
                    .cmp(&rb.function)
                    .then(rb.block.cmp(&ra.block))
                    .then(a.cmp(b)),
            }
        })?
        .clone();
    queue.remove(&key);
    Some(key)
}
fn general(
    model: &Model,
    budget: Budget,
    reason: &str,
    visits: usize,
) -> (BTreeMap<RowKey, Node>, Option<String>, usize) {
    let successors: Vec<_> = (0..model.rows.len())
        .map(|pc| Edge {
            target: Some(RowKey { pc, calls: vec![] }),
            fault: None,
        })
        .chain([
            Edge {
                target: None,
                fault: Some(None),
            },
            Edge {
                target: None,
                fault: None,
            },
        ])
        .collect();
    let successors = Arc::new(successors);
    let values = model
        .initial
        .iter()
        .map(|v| KnownBits::unknown(v.0.len()))
        .collect::<Vec<_>>();
    let values = Arc::new(values);
    let nodes = (0..model.rows.len())
        .map(|pc| {
            let key = RowKey { pc, calls: vec![] };
            let event = general_event(model, pc, budget);
            (
                key.clone(),
                Node {
                    key,
                    values: values.clone(),
                    operands: model.operands(pc, &values),
                    successors: successors.clone(),
                    event,
                    source_charge: 1,
                    general_kernel: true,
                },
            )
        })
        .collect();
    (nodes, Some(reason.into()), visits)
}
fn general_event(model: &Model, pc: usize, budget: Budget) -> Event {
    match model.rows[pc].public.opcode.as_str() {
        "lift.call" => Event::Call {
            cells: 3 + model.locals[model.call_name(pc)].len() as u64,
            stack: Interval {
                min: 0,
                max: budget.stack,
            },
            depth: None,
            check: Check::Dynamic,
        },
        "lift.ret" => Event::Return { depth: None },
        _ => Event::General,
    }
}
