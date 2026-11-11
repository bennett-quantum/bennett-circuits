use super::*;
pub(super) struct Effects {
    pub writes: Vec<(usize, KnownBits)>,
    pub fault: Option<bool>,
    pub codes: Vec<u8>,
    pub general: bool,
    pub havoc: bool,
}
pub(super) fn evaluate(model: &Model, pc: usize, before: &[KnownBits]) -> Effects {
    let row = &model.rows[pc];
    let i = &row.instruction;
    let op = i.opcode.strip_prefix("lift.").unwrap_or(&i.opcode);
    let mut effects = Effects {
        writes: vec![],
        fault: Some(false),
        codes: vec![],
        general: false,
        havoc: false,
    };
    if op == "parmov" {
        parallel(model, pc, before, &mut effects);
        return effects;
    }
    let ids: Vec<_> = i
        .lhs
        .iter()
        .map(|n| model.id(&row.public.function, n))
        .collect();
    let width = ids.first().map_or(0, |id| model.locations[*id].width);
    let values = model.operands(pc, before);
    if let Some(value) = pure(model, pc, op, &values, width) {
        if let Some(id) = ids.first() {
            effects.writes.push((*id, value));
        }
    } else {
        general(op, &ids, model, &values, &mut effects);
    }
    effects
}
fn pure(
    model: &Model,
    pc: usize,
    op: &str,
    values: &[KnownBits],
    width: usize,
) -> Option<KnownBits> {
    let row = &model.rows[pc];
    if op.starts_with("const.i") {
        let Operand::Int(text) = &row.instruction.operands[0] else {
            panic!("checked integer")
        };
        return Some(KnownBits::integer(text, width));
    }
    if op.starts_with("zext.i") || op.starts_with("sext.i") || op.starts_with("trunc.i") {
        return Some(values[0].resize(width, op.starts_with("sext")));
    }
    if let Some(predicate) = op.strip_prefix("icmp.") {
        return Some(values[0].compare(&values[1], predicate));
    }
    match op {
        "mov" => Some(values[0].resize(width, false)),
        "label" => Some(KnownBits::integer(
            &model
                .label(&row.public.function, &row.instruction.operands[0])
                .to_string(),
            width,
        )),
        "addr" => Some(KnownBits::unknown(width)),
        "select" => Some(KnownBits::select(values[0].0[0], &values[1], &values[2])),
        "add" | "sub" => Some(values[0].add(&values[1], op == "sub")),
        "and" | "or" | "xor" => Some(values[0].bitwise(&values[1], op)),
        "mul" => Some(values[0].multiply(&values[1])),
        "lshr" | "ashr" | "shl" => Some(
            values[1]
                .as_u128()
                .map_or_else(|| KnownBits::unknown(width), |n| values[0].shift(n, op)),
        ),
        _ => None,
    }
}
fn parallel(model: &Model, pc: usize, before: &[KnownBits], effects: &mut Effects) {
    let row = &model.rows[pc];
    let Operand::List(names) = &row.instruction.operands[0] else {
        panic!("checked parmov")
    };
    for (name, destination) in names.iter().zip(&row.instruction.lhs) {
        let id = model.id(&row.public.function, destination);
        effects.writes.push((
            id,
            model.read(&row.public.function, &Operand::Name(name.clone()), before),
        ));
    }
}
fn general(op: &str, ids: &[usize], model: &Model, values: &[KnownBits], effects: &mut Effects) {
    effects.general = true;
    effects.fault = None;
    if matches!(op, "udiv" | "urem" | "sdiv" | "srem") {
        effects.fault = values[1].equal(&KnownBits::integer("0", values[1].0.len()));
        effects.codes.push(5);
        if op.starts_with('s') {
            effects.fault = None;
            effects.codes.push(6);
        }
    } else {
        // Memory tables, nested actions/oracles and any unproved effects use
        // general kernels. Forget every possible written location, including
        // in-place operands absent from lhs. No read-only proof is invented.
        effects.havoc = true;
    }
    for id in ids {
        effects
            .writes
            .push((*id, KnownBits::unknown(model.locations[*id].width)));
    }
}
