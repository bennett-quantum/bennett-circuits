//! Source-point translation witnesses, ISA 0.9-draft §12.1 / §5.6.
//! Wire references use the source's §16 scalar wire names; points are the
//! published `[interface_hash, block, ordinal]` arrays (§9.2).
use crate::{
    Result,
    flatten::FlatGate,
    ir::BaseGate,
    json::Value as V,
    object::{Function, Object, Operand, Type},
};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) fn witnesses(o: &Object, f: &Function) -> Result<Vec<V>> {
    let hash = &o.identity()?[&f.name].interface_hash;
    let mut names = BTreeMap::new();
    for p in f.groups[..2].iter().flatten() {
        if p.ty != Type::Wire(1) {
            return Ok(vec![]);
        }
        let n = names.len();
        names.insert(p.name.clone(), n);
    }
    let mut borrowed = BTreeSet::new();
    let mut gates = vec![];
    let mut points = vec![];
    for block in &f.blocks {
        for (ordinal, i) in block.instrs.iter().enumerate() {
            if !i.children.is_empty() {
                return Ok(vec![]);
            }
            match i.opcode.as_str() {
                "qborrow" => {
                    if i.lhs.len() != 1 || i.operands.last() != Some(&Operand::Int("1".into())) {
                        return Ok(vec![]);
                    }
                    let n = names.len();
                    names.insert(i.lhs[0].clone(), n);
                    borrowed.insert(n);
                }
                "qreturn" => {}
                "x" | "cx" | "ccx" => {
                    let mut ws = vec![];
                    for a in &i.operands {
                        let Operand::Name(n) = a else {
                            return Ok(vec![]);
                        };
                        let Some(q) = names.get(n) else {
                            return Ok(vec![]);
                        };
                        ws.push(*q);
                    }
                    if ws.len()
                        != match i.opcode.as_str() {
                            "x" => 1,
                            "cx" => 2,
                            _ => 3,
                        }
                    {
                        return Ok(vec![]);
                    }
                    let target = ws.pop().unwrap();
                    gates.push(FlatGate {
                        gate: BaseGate::X,
                        controls: ws.into_iter().map(|q| (q, true)).collect(),
                        targets: vec![target],
                    });
                    points.push(V::Array(vec![
                        V::Str(hash.clone()),
                        V::Str(block.name.clone()),
                        V::Int(ordinal.to_string()),
                    ]));
                }
                _ => return Ok(vec![]),
            }
        }
    }
    if borrowed.is_empty() {
        return Ok(vec![]);
    }
    let touches = |g: &FlatGate| {
        g.targets
            .iter()
            .chain(g.controls.iter().map(|(q, _)| q))
            .any(|q| borrowed.contains(q))
    };
    let Some(first) = gates.iter().position(touches) else {
        return Ok(vec![]);
    };
    let last = gates.iter().rposition(touches).unwrap();
    let frame = &gates[first..=last];
    if crate::checker::translation_component(frame, &borrowed).is_err() {
        return Ok(vec![]);
    }
    let vlen = frame
        .iter()
        .take_while(|g| borrowed.contains(&g.targets[0]))
        .count();
    if vlen == 0 || frame.len() % 2 != 0 || frame.len() / 2 <= vlen {
        return Ok(vec![]);
    }
    let wlen = frame.len() / 2 - vlen;
    let x = frame[..vlen]
        .iter()
        .flat_map(|g| g.controls.iter().map(|(q, _)| *q))
        .collect::<BTreeSet<_>>();
    let wires = |set: &BTreeSet<usize>| {
        V::Array(
            set.iter()
                .map(|q| V::Str(names.iter().find(|(_, v)| *v == q).unwrap().0.clone()))
                .collect(),
        )
    };
    let witness = crate::certificate::obj([
        ("V", V::Array(points[first..first + vlen].to_vec())),
        (
            "W",
            V::Array(points[first + vlen..first + vlen + wlen].to_vec()),
        ),
        ("X", wires(&x)),
        ("B", wires(&borrowed)),
        (
            "frame",
            V::Array(vec![points[first].clone(), points[last].clone()]),
        ),
    ]);
    Ok(vec![witness])
}
