//! Selector immutability is a syntactic obligation, including writes later
//! undone. It cannot be inferred from net equality after loop unrolling.
use crate::{
    Result, bail,
    object::{Function, Instr, Object, Operand},
};
use std::collections::{BTreeMap, BTreeSet};
type Ref = (String, Option<u32>);
fn reference(o: &Operand) -> Option<Ref> {
    match o {
        Operand::Name(n) => Some((n.clone(), None)),
        Operand::Indexed(n, i) => Some((n.clone(), Some(*i))),
        _ => None,
    }
}
fn overlaps(a: &Ref, b: &Ref) -> bool {
    a.0 == b.0 && (a.1.is_none() || b.1.is_none() || a.1 == b.1)
}
fn writes(o: &Object, i: &Instr) -> Vec<Ref> {
    let mut head = i.opcode.as_str();
    let mut args = i.operands.as_slice();
    while head == "adj" || head == "ctl" {
        let offset = usize::from(head == "ctl");
        let Some(Operand::Name(next)) = args.get(offset) else {
            return Vec::new();
        };
        head = next;
        args = &args[offset + 1..];
    }
    let indices: &[usize] = match head {
        "x" | "h" | "s" | "sdg" | "t" | "tdg" => &[0],
        "rx" | "ry" | "rz" | "cx" | "qxor" | "mcx" => &[1],
        "ccx" => &[2],
        "swap" | "qexch" => &[0, 1],
        "cz" => &[0, 1],
        "qstore" => &[0],
        "qexchm" => &[0, 2],
        "qinplace" => &[3],
        "oracle_call" => &[3, 4],
        // Consuming a selector is also prohibited in its active context.
        "qfree" | "qreturn" => &[0],
        "quncopy" => &[1],
        "qunload" => &[2],
        _ => &[],
    };
    let mut out = indices
        .iter()
        .filter_map(|k| args.get(*k).and_then(reference))
        .collect::<Vec<_>>();
    if head == "action_call" {
        if let Some(Operand::List(ns)) = args.get(1) {
            out.extend(ns.iter().map(|n| (n.clone(), None)));
        }
        if let (Some(Operand::Sym(n)), Some(Operand::List(ns))) = (args.first(), args.get(3)) {
            let sig = o.symbol(n).map(|s| &s.signature).or_else(|| {
                o.imports
                    .iter()
                    .find(|s| s.name == *n)
                    .map(|s| &s.signature)
            });
            if let Some(ms) = sig.and_then(|s| s.get("mem")).and_then(|v| v.as_array()) {
                out.extend(
                    ns.iter()
                        .zip(ms)
                        .filter(|(_, m)| m.get("role").and_then(|v| v.as_str()) == Some("mut"))
                        .map(|(n, _)| (n.clone(), None)),
                );
            }
        }
    }
    out
}
fn visit(
    o: &Object,
    is: &[Instr],
    deps: &mut BTreeMap<String, BTreeSet<Ref>>,
    protected: &[Ref],
) -> Result<()> {
    for i in is {
        let written = writes(o, i);
        if written
            .iter()
            .any(|w| protected.iter().any(|p| overlaps(w, p)))
        {
            bail!("checker: selector or while-condition dependency modified/consumed (§3.3; §4.6)");
        }
        let mut inner = protected.to_vec();
        let selector = match i.opcode.as_str() {
            "qif" => i.operands.first(),
            "qloop" if i.operands.len() == 3 => i.operands.get(2),
            _ => None,
        };
        if let Some(selector) = selector.and_then(reference) {
            inner.push(selector.clone());
            if i.opcode == "qloop" {
                inner.extend(deps.get(&selector.0).into_iter().flatten().cloned());
            }
        }
        for child in &i.children {
            visit(o, child, deps, &inner)?;
        }
        let mut reads = BTreeSet::new();
        for a in &i.operands {
            let rs = match a {
                Operand::List(ns) => ns.iter().map(|n| (n.clone(), None)).collect(),
                _ => reference(a).into_iter().collect::<Vec<_>>(),
            };
            for r in rs {
                reads.extend(deps.get(&r.0).into_iter().flatten().cloned());
                reads.insert(r);
            }
        }
        if matches!(i.opcode.as_str(), "qcopy" | "qload") {
            for n in &i.lhs {
                deps.insert(n.clone(), reads.clone());
            }
        } else if i.opcode == "qalloc" {
            for n in &i.lhs {
                deps.insert(n.clone(), BTreeSet::new());
            }
        } else {
            for (n, _) in written {
                deps.entry(n).or_default().extend(reads.clone());
            }
        }
    }
    Ok(())
}
pub(crate) fn validate(o: &Object, f: &Function) -> Result<()> {
    let mut deps = BTreeMap::new();
    for b in &f.blocks {
        visit(o, &b.instrs, &mut deps, &[])?;
    }
    Ok(())
}
