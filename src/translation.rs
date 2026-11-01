//! Source-point translation witnesses, ISA 0.9-draft §12.1 / §5.6.
//! Wire references use §16 names/indexed wires; points use §9.2 arrays.
use crate::{
    Result,
    flatten::FlatGate,
    ir::BaseGate,
    json::Value as V,
    object::{Function, Object, Operand, Type},
};
use std::collections::{BTreeMap, BTreeSet};
fn wire(names: &BTreeMap<String, Vec<usize>>, operand: &Operand) -> Option<usize> {
    match operand {
        Operand::Name(n) => names.get(n).filter(|ws| ws.len() == 1).map(|ws| ws[0]),
        Operand::Indexed(n, i) => names.get(n)?.get(*i as usize).copied(),
        _ => None,
    }
}
fn named_wire(names: &BTreeMap<String, Vec<usize>>, n: &str) -> Option<usize> {
    if let Some((base, index)) = n.strip_suffix(']').and_then(|s| s.rsplit_once('[')) {
        return names.get(base)?.get(index.parse::<usize>().ok()?).copied();
    }
    wire(names, &Operand::Name(n.into()))
}
pub(crate) fn witnesses(o: &Object, f: &Function) -> Result<Vec<V>> {
    let hash = &o.identity()?[&f.name].interface_hash;
    let mut names = BTreeMap::<String, Vec<usize>>::new();
    let mut labels = BTreeMap::<usize, String>::new();
    let mut next = 0usize;
    let mut declare = |names: &mut BTreeMap<String, Vec<usize>>, name: &str, width: u32| {
        let ws = (next..next + width as usize).collect::<Vec<_>>();
        for (i, q) in ws.iter().enumerate() {
            labels.insert(
                *q,
                if width == 1 {
                    name.to_string()
                } else {
                    format!("{name}[{i}]")
                },
            );
        }
        next += width as usize;
        names.insert(name.into(), ws);
    };
    for p in f.groups[..2].iter().flatten() {
        let Type::Wire(w) = p.ty else {
            return Ok(vec![]);
        };
        declare(&mut names, &p.name, w);
    }
    let mut borrowed = BTreeSet::new();
    let mut gates = vec![];
    let mut points = vec![];
    let mut controls = Vec::<(usize, bool)>::new();
    for block in &f.blocks {
        for (ordinal, i) in block.instrs.iter().enumerate() {
            if !i.children.is_empty() {
                return Ok(vec![]);
            }
            let point = V::Array(vec![
                V::Str(hash.clone()),
                V::Str(block.name.clone()),
                V::Int(ordinal.to_string()),
            ]);
            let mut emit = |native: Vec<usize>, target: usize| {
                gates.push(FlatGate {
                    gate: BaseGate::X,
                    controls: native
                        .into_iter()
                        .map(|q| (q, true))
                        .chain(controls.iter().copied())
                        .collect(),
                    targets: vec![target],
                });
                points.push(point.clone());
            };
            match i.opcode.as_str() {
                "qborrow" => {
                    let Some(Operand::Int(width)) = i.operands.last() else {
                        return Ok(vec![]);
                    };
                    let Ok(width) = width.parse::<u32>() else {
                        return Ok(vec![]);
                    };
                    if i.lhs.len() != 1 || width == 0 {
                        return Ok(vec![]);
                    }
                    declare(&mut names, &i.lhs[0], width);
                    borrowed.extend(&names[&i.lhs[0]]);
                }
                "qreturn" => {}
                "qpred" => {
                    let Some(q) = i.operands.first().and_then(|v| wire(&names, v)) else {
                        return Ok(vec![]);
                    };
                    controls.push((q, true));
                }
                "qunpred" => {
                    let Some(q) = i.operands.first().and_then(|v| wire(&names, v)) else {
                        return Ok(vec![]);
                    };
                    if controls.pop() != Some((q, true)) {
                        return Ok(vec![]);
                    }
                }
                "qxor" => {
                    let [Operand::Name(src), Operand::Name(dst)] = i.operands.as_slice() else {
                        return Ok(vec![]);
                    };
                    let (Some(src), Some(dst)) = (names.get(src), names.get(dst)) else {
                        return Ok(vec![]);
                    };
                    if src.len() != dst.len() {
                        return Ok(vec![]);
                    }
                    for (src, dst) in src.iter().zip(dst) {
                        emit(vec![*src], *dst);
                    }
                }
                "x" | "cx" | "ccx" => {
                    let Some(mut ws) = i
                        .operands
                        .iter()
                        .map(|a| wire(&names, a))
                        .collect::<Option<Vec<_>>>()
                    else {
                        return Ok(vec![]);
                    };
                    let expected = match i.opcode.as_str() {
                        "x" => 1,
                        "cx" => 2,
                        _ => 3,
                    };
                    if ws.len() != expected {
                        return Ok(vec![]);
                    }
                    let target = ws.pop().unwrap();
                    emit(ws, target);
                }
                "mcx" => {
                    let [Operand::List(cs), target] = i.operands.as_slice() else {
                        return Ok(vec![]);
                    };
                    let (Some(cs), Some(target)) = (
                        cs.iter()
                            .map(|n| named_wire(&names, n))
                            .collect::<Option<Vec<_>>>(),
                        wire(&names, target),
                    ) else {
                        return Ok(vec![]);
                    };
                    emit(cs, target);
                }
                _ => return Ok(vec![]),
            }
        }
    }
    if !controls.is_empty() || borrowed.is_empty() {
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
    let wires =
        |set: &BTreeSet<usize>| V::Array(set.iter().map(|q| V::Str(labels[q].clone())).collect());
    // A bundle instruction expands into several gates, but appears once in
    // the source-point list. Boundaries cannot cut an instruction expansion.
    if points[first..first + vlen].last() == points.get(first + vlen)
        || points[first + vlen..first + vlen + wlen].last() == points.get(first + vlen + wlen)
    {
        return Ok(vec![]);
    }
    let source_points = |ps: &[V]| {
        let mut ps = ps.to_vec();
        ps.dedup();
        V::Array(ps)
    };
    Ok(vec![crate::certificate::obj([
        ("V", source_points(&points[first..first + vlen])),
        (
            "W",
            source_points(&points[first + vlen..first + vlen + wlen]),
        ),
        ("X", wires(&x)),
        ("B", wires(&borrowed)),
        (
            "frame",
            V::Array(vec![points[first].clone(), points[last].clone()]),
        ),
    ])])
}
