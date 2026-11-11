use super::*;
use crate::object::Operand;
use flow::State;
use model::Model;
mod effects;
pub(super) struct Transition {
    pub node: Node,
    pub states: Vec<(RowKey, State)>,
}
impl Transition {
    fn go(&mut self, target: RowKey, state: State) {
        Arc::make_mut(&mut self.node.successors).push(Edge {
            target: Some(target.clone()),
            fault: None,
        });
        self.states.push((target, state));
    }
    fn halt(&mut self, fault: Option<Option<u8>>) {
        Arc::make_mut(&mut self.node.successors).push(Edge {
            target: None,
            fault,
        });
    }
}
pub(super) fn execute(model: &Model, key: &RowKey, before: &State, budget: Budget) -> Transition {
    let source = &model.rows[key.pc];
    let node = Node {
        key: key.clone(),
        values: Arc::new(before.values.clone()),
        operands: model.operands(key.pc, &before.values),
        successors: Arc::new(vec![]),
        event: Event::Row,
        source_charge: 1,
        general_kernel: false,
    };
    let mut out = Transition {
        node,
        states: vec![],
    };
    match source
        .public
        .opcode
        .strip_prefix("lift.")
        .unwrap_or(&source.public.opcode)
    {
        "call" => call(model, key, before, budget, &mut out),
        "ret" => ret(model, key, before, &mut out),
        "retval" => {
            out.node.event = Event::Success;
            out.halt(None);
        }
        "qjump" | "qbranch" => branch(model, key, before, &mut out),
        "qjumpr" => indirect(model, key, before, &mut out),
        "fault" | "faultif" => fault(model, key, before, &mut out),
        _ => ordinary(model, key, before, &mut out),
    }
    Arc::make_mut(&mut out.node.successors).sort();
    Arc::make_mut(&mut out.node.successors).dedup();
    out
}
fn call(model: &Model, key: &RowKey, before: &State, budget: Budget, out: &mut Transition) {
    let callee = model.call_name(key.pc);
    let cells = 3 + model.locals[callee].len() as u64;
    let depth = key.calls.len() as u64;
    let fault = depth >= budget.depth
        || u128::from(before.stack.min) + u128::from(cells) > u128::from(budget.stack);
    let safe = depth < budget.depth
        && u128::from(before.stack.max) + u128::from(cells) <= u128::from(budget.stack);
    out.node.event = Event::Call {
        cells,
        stack: before.stack,
        depth: Some(depth),
        check: if fault {
            Check::Fault
        } else if safe {
            Check::Safe
        } else {
            Check::Dynamic
        },
    };
    if !safe {
        out.halt(Some(Some(3)));
    }
    if fault {
        return;
    }
    let mut state = before.clone();
    state.saved.push(before.values.clone());
    for id in &model.locals[callee] {
        state.values[*id] = KnownBits::integer("0", model.locations[*id].width);
    }
    for (id, value) in model.parameters[callee].iter().zip(&out.node.operands) {
        state.values[*id] = value.clone();
    }
    state.stack = Interval {
        min: before.stack.min + cells,
        max: (u128::from(before.stack.max) + u128::from(cells)).min(u128::from(budget.stack))
            as u64,
    };
    let mut calls = key.calls.clone();
    calls.push(key.pc);
    out.go(
        RowKey {
            pc: model.entries[callee],
            calls,
        },
        state,
    );
}
fn ret(model: &Model, key: &RowKey, before: &State, out: &mut Transition) {
    out.node.event = Event::Return {
        depth: Some(key.calls.len() as u64),
    };
    let mut calls = key.calls.clone();
    let Some(site) = calls.pop() else {
        out.halt(Some(Some(4)));
        return;
    };
    let f = &model.rows[key.pc].public.function;
    let caller = &model.rows[site];
    let destination = model.id(&caller.public.function, &caller.instruction.lhs[0]);
    let mut state = before.clone();
    let saved = state.saved.pop().expect("call context has saved locations");
    for id in &model.locals[f] {
        state.values[*id] = saved[*id].clone();
    }
    state.values[destination] = out.node.operands[0].clone();
    out.go(
        RowKey {
            pc: site + 1,
            calls,
        },
        state,
    );
}
fn branch(model: &Model, key: &RowKey, before: &State, out: &mut Transition) {
    let row = &model.rows[key.pc];
    let i = &row.instruction;
    if i.opcode == "qjump" {
        out.go(
            RowKey {
                pc: model.label(&row.public.function, &i.operands[0]),
                calls: key.calls.clone(),
            },
            before.clone(),
        );
        return;
    }
    let bit = out.node.operands[0].0[0];
    for (condition, operand) in [(true, &i.operands[1]), (false, &i.operands[2])] {
        if bit.is_none_or(|v| v == condition) {
            out.go(
                RowKey {
                    pc: model.label(&row.public.function, operand),
                    calls: key.calls.clone(),
                },
                before.clone(),
            );
        }
    }
}
fn indirect(model: &Model, key: &RowKey, before: &State, out: &mut Transition) {
    let function = &model.rows[key.pc].public.function;
    let value = out.node.operands[0].as_u128();
    for row in &model.rows {
        if row.public.function == *function && value.is_none_or(|v| v == row.public.pc as u128) {
            out.go(
                RowKey {
                    pc: row.public.pc,
                    calls: key.calls.clone(),
                },
                before.clone(),
            );
        }
    }
    let valid = value.is_some_and(|v| {
        model
            .rows
            .iter()
            .any(|r| r.public.pc as u128 == v && r.public.function == *function)
    });
    if !valid {
        out.halt(Some(Some(4)));
    }
}
fn fault(model: &Model, key: &RowKey, before: &State, out: &mut Transition) {
    let row = &model.rows[key.pc].instruction;
    let conditional = row.opcode == "lift.faultif";
    let bit = if conditional {
        out.node.operands[0].0[0]
    } else {
        Some(true)
    };
    let operand = &row.operands[usize::from(conditional)];
    let Operand::Int(code) = operand else {
        panic!("checked fault code")
    };
    if bit != Some(false) {
        out.halt(Some(KnownBits::integer(code, 4).as_u128().map(|v| v as u8)));
    }
    if bit != Some(true) {
        out.go(
            RowKey {
                pc: key.pc + 1,
                calls: key.calls.clone(),
            },
            before.clone(),
        );
    }
}
fn ordinary(model: &Model, key: &RowKey, before: &State, out: &mut Transition) {
    let effect = effects::evaluate(model, key.pc, &before.values);
    out.node.general_kernel = effect.general;
    if effect.fault != Some(false) {
        if effect.codes.is_empty() {
            out.halt(Some(None));
        }
        for code in &effect.codes {
            out.halt(Some(Some(*code)));
        }
    }
    if effect.fault == Some(true) {
        return;
    }
    let mut state = before.clone();
    if effect.havoc {
        for value in &mut state.values {
            *value = KnownBits::unknown(value.0.len());
        }
    }
    for (id, value) in effect.writes {
        state.values[id] = value;
    }
    out.go(
        RowKey {
            pc: key.pc + 1,
            calls: key.calls.clone(),
        },
        state,
    );
}
