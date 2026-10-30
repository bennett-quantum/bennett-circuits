//! Version 1 symbolic instruction histories (ratified 7bu, ISA §6.2).
//! Atoms are formal arguments/tokens, never representative runtime values.
use crate::{
    Error, Result, bail,
    object::{Function, Object, Operand, Type},
};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: u32 = 1;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingObligation {
    pub import: String,
    pub rule: String,
}
/// Import bodies and their transitive proof contracts are discharged only by
/// §9.3 resolution and validation, even for a pinned import.
pub fn pending(o: &Object) -> Vec<PendingObligation> {
    o.imports
        .iter()
        .map(|i| PendingObligation {
            import: i.name.clone(),
            rule: "§9.3 body validation and §12.3 certificate composition (bennett-circuits-z9r)"
                .into(),
        })
        .collect()
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Term {
    Atom(String),
    Apply(String, Vec<Term>),
    Permutation(String, Vec<Term>),
    Oracle(String, Vec<Term>),
    Xor(BTreeSet<Term>),
}
fn quantum(t: &Term) -> bool {
    match t {
        Term::Apply(..) => true,
        Term::Xor(ts) => ts.iter().any(quantum),
        Term::Oracle(_, ts) | Term::Permutation(_, ts) => ts.iter().any(quantum),
        _ => false,
    }
}
fn zero() -> Term {
    Term::Xor(BTreeSet::new())
}
fn xor(a: Term, b: Term) -> Term {
    let mut terms = BTreeSet::new();
    for t in [a, b] {
        let xs = if let Term::Xor(xs) = t {
            xs
        } else {
            BTreeSet::from([t])
        };
        for x in xs {
            if !terms.insert(x.clone()) {
                terms.remove(&x);
            }
        }
    }
    if terms.len() == 1 {
        terms.pop_first().unwrap()
    } else {
        Term::Xor(terms)
    }
}
fn name(o: &Operand) -> Result<&str> {
    if let Operand::Name(n) = o {
        Ok(n)
    } else {
        bail!("history: expected bundle (§3.3)")
    }
}
fn list(o: &Operand) -> Result<&[String]> {
    if let Operand::List(n) = o {
        Ok(n)
    } else {
        bail!("history: expected list (§3.3)")
    }
}
type Context = Vec<(String, bool)>;
fn prepare(
    is: &[crate::object::Instr],
    inherited: &Context,
    out: &mut Vec<(crate::object::Instr, Context)>,
) -> Result<bool> {
    let mut active = inherited.clone();
    let mut pushed = vec![];
    for i in is {
        if matches!(i.opcode.as_str(), "qpred" | "qunpred") {
            if i.operands.len() != 1 || !i.lhs.is_empty() {
                bail!("history: predicate schema (§3.3)")
            }
            let n = name(&i.operands[0])?.to_string();
            if i.opcode == "qpred" {
                if active.iter().any(|(c, _)| *c == n) {
                    bail!("history: duplicate control (§3.3)")
                }
                active.push((n.clone(), true));
                pushed.push(n);
            } else {
                if pushed.pop().as_deref() != Some(&n) || active.pop() != Some((n, true)) {
                    bail!("history: predicate stack (§3.5)")
                }
            }
            continue;
        }
        if i.opcode == "qif" {
            if i.operands.len() != 1
                || i.children.is_empty()
                || i.children.len() > 2
                || !i.lhs.is_empty()
            {
                bail!("history: qif schema (§4.6)")
            }
            let n = name(&i.operands[0])?.to_string();
            for (j, body) in i.children.iter().enumerate() {
                let mut cs = active.clone();
                cs.push((n.clone(), j == 0));
                if !prepare(body, &cs, out)? {
                    return Ok(false);
                }
            }
            continue;
        }
        if i.opcode == "qloop" {
            if i.operands.len() != 1 || i.children.len() != 1 {
                return Ok(false);
            }
            let Some(Operand::Int(n)) = i.operands.first() else {
                return Ok(false);
            };
            let n = n
                .parse::<u64>()
                .map_err(|_| Error("host limit: history loop count".into()))?;
            for _ in 0..n {
                if !prepare(&i.children[0], &active, out)? {
                    return Ok(false);
                }
            }
            continue;
        }
        let mut plain = i.clone();
        let mut cs = active.clone();
        let mut adj = false;
        let mut controlled = false;
        while matches!(plain.opcode.as_str(), "ctl" | "adj") {
            let count = if plain.opcode == "ctl" {
                if controlled {
                    bail!("history: repeated ctl modifier (§4.7)")
                }
                controlled = true;
                let n = name(plain.operands.first().ok_or("history: ctl operand")?)?.to_string();
                cs.push((n, true));
                1
            } else {
                if adj {
                    bail!("history: double adj (§4.7)")
                }
                adj = true;
                0
            };
            let Some(Operand::Name(next)) = plain.operands.get(count) else {
                bail!("history: call modifier (§4.7)")
            };
            plain.opcode = next.clone();
            plain.operands.drain(..count + 1);
        }
        if (adj || matches!(i.opcode.as_str(), "ctl" | "adj"))
            && !matches!(plain.opcode.as_str(), "action_call" | "oracle_call")
        {
            bail!("history: modifiers require region calls (§4.7)")
        }
        if cs.len() != cs.iter().map(|(n, _)| n).collect::<BTreeSet<_>>().len() {
            bail!("history: duplicate effective control (§3.3)")
        }
        if adj {
            plain
                .operands
                .insert(0, Operand::Name(plain.opcode.clone()));
            plain.opcode = "adj".into();
        }
        out.push((plain, cs));
    }
    if !pushed.is_empty() {
        bail!("history: predicate stack at exit (§3.5)")
    }
    Ok(true)
}
fn masked(value: Term, condition: &Term) -> Term {
    if value == zero() {
        zero()
    } else if *condition == Term::Atom("one".into()) {
        value
    } else {
        Term::Oracle("mask".into(), vec![condition.clone(), value])
    }
}
/// Returns false only when another universal checker must handle this body.
/// An unresolved body is deliberately pending, never a completed proof.
pub(crate) fn verify(o: &Object, f: &Function) -> Result<bool> {
    if f.blocks.iter().any(|b| !b.params.is_empty()) {
        bail!("history: block arguments have no branch binding (§5.3)")
    }
    let source = f
        .blocks
        .iter()
        .flat_map(|b| b.instrs.clone())
        .collect::<Vec<_>>();
    let mut prepared = vec![];
    if !prepare(&source, &vec![], &mut prepared)? {
        return Ok(false);
    }
    let instructions = prepared.iter().map(|(i, _)| i).collect::<Vec<_>>();
    let supported = [
        "qalloc",
        "qcopy",
        "qload",
        "qunload",
        "quncopy",
        "qfree",
        "x",
        "h",
        "s",
        "sdg",
        "t",
        "tdg",
        "rx",
        "ry",
        "rz",
        "qxor",
        "action_call",
        "oracle_call",
        "adj",
    ];
    if instructions
        .iter()
        .any(|i| !supported.contains(&i.opcode.as_str()) || !i.children.is_empty())
    {
        return Ok(false);
    }
    // This checker complements the bit-level algebra. Whole-bundle histories
    // deliberately do not alias an indexed operand with its enclosing bundle.
    if instructions
        .iter()
        .any(|i| i.operands.iter().any(|a| matches!(a, Operand::Indexed(..))))
    {
        return Ok(false);
    }
    let mut values = BTreeMap::<String, Term>::new();
    let mut widths = BTreeMap::new();
    let mut local = BTreeSet::new();
    let mut types = f
        .groups
        .iter()
        .flatten()
        .map(|p| (p.name.clone(), p.ty.clone()))
        .collect::<BTreeMap<_, _>>();
    for p in f.groups[..3].iter().flatten() {
        values.insert(p.name.clone(), Term::Atom(p.name.clone()));
        widths.insert(p.name.clone(), p.ty.clone());
    }
    let mut deferred = false;
    let mut copies = BTreeMap::<String, String>::new();
    let mut loads = BTreeMap::<String, (String, String)>::new();
    let mut dependencies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut producer_dependencies = BTreeMap::<Term, BTreeSet<String>>::new();
    let mut allocation_ids = BTreeMap::<String, u64>::new();
    let mut next_allocation = 0u64;
    let mut closed = BTreeSet::new();
    let read = |vs: &BTreeMap<String, Term>, n: &str| -> Result<Term> {
        vs.get(n)
            .cloned()
            .ok_or_else(|| Error(format!("history: non-live bundle {n} (§3.3)")))
    };
    for (i, controls) in prepared {
        let mut cs = vec![];
        for (n, positive) in &controls {
            if widths.get(n) != Some(&Type::Wire(1)) {
                bail!("history: control must be a live wire (§3.3)")
            }
            let value = read(&values, n)?;
            if quantum(&value) {
                return Ok(false);
            }
            cs.push(if *positive {
                value
            } else {
                xor(value, Term::Atom("one".into()))
            });
        }
        let condition = if cs.is_empty() {
            Term::Atom("one".into())
        } else {
            Term::Oracle("condition".into(), cs)
        };

        let mut op = i.opcode.as_str();
        let mut args = i.operands.as_slice();
        let mut adj = false;
        if op == "adj" {
            adj = true;
            let Some(Operand::Name(n)) = args.first() else {
                bail!("history: adj schema (§3.3)")
            };
            op = n;
            args = &args[1..];
        }
        if matches!(op, "qcopy" | "quncopy" | "qxor" | "qfree") {
            for a in args {
                if let Operand::Name(n) = a
                    && controls.iter().any(|(c, _)| c == n)
                {
                    bail!("history: effective control overlaps bundle operand (§3.3)")
                }
            }
        }
        match op {
            "qload" => {
                if args.len() != 2 || i.lhs.len() != 1 {
                    bail!("history: qload arity/result (§3.3)")
                }
                let region = name(&args[0])?;
                let addr = name(&args[1])?;
                let dst = &i.lhs[0];
                let Some(Type::Region(a, w)) = widths.get(region).cloned() else {
                    bail!("history: qload region (§3.3)")
                };
                if widths.get(addr) != Some(&Type::Wire(a))
                    || values.contains_key(dst)
                    || controls.iter().any(|(n, _)| n == region || n == addr)
                {
                    bail!("history: qload width/liveness/disjointness (§3.3)")
                }
                if quantum(&read(&values, region)?) || quantum(&read(&values, addr)?) {
                    return Ok(false);
                }
                let value = Term::Oracle(
                    "load".into(),
                    vec![
                        read(&values, region)?,
                        read(&values, addr)?,
                        condition.clone(),
                    ],
                );
                allocation_ids.insert(dst.clone(), next_allocation);
                next_allocation = next_allocation
                    .checked_add(1)
                    .ok_or("host limit: symbolic allocation identity")?;
                values.insert(dst.clone(), value);
                widths.insert(dst.clone(), Type::Wire(w));
                types.insert(dst.clone(), Type::Wire(w));
                local.insert(dst.clone());
                loads.insert(dst.clone(), (region.into(), addr.into()));
                dependencies.insert(
                    dst.clone(),
                    [region.to_string(), addr.to_string()]
                        .into_iter()
                        .chain(controls.iter().map(|(n, _)| n.clone()))
                        .collect(),
                );
            }
            "qunload" => {
                if args.len() != 3 || !i.lhs.is_empty() {
                    bail!("history: qunload schema (§3.3)")
                }
                let region = name(&args[0])?;
                let addr = name(&args[1])?;
                let dst = name(&args[2])?;
                if region == dst
                    || addr == dst
                    || !local.contains(dst)
                    || controls
                        .iter()
                        .any(|(n, _)| n == region || n == addr || n == dst)
                {
                    bail!("history: qunload liveness/disjointness (§3.3)")
                }
                let Some(Type::Region(a, w)) = widths.get(region).cloned() else {
                    bail!("history: qunload region (§3.3)")
                };
                if widths.get(addr) != Some(&Type::Wire(a))
                    || widths.get(dst) != Some(&Type::Wire(w))
                {
                    bail!("history: qunload widths (§3.3)")
                }
                if loads.get(dst) != Some(&(region.into(), addr.into())) {
                    bail!("history: qunload producer differs (§6.2)")
                }
                let expected = Term::Oracle(
                    "load".into(),
                    vec![
                        read(&values, region)?,
                        read(&values, addr)?,
                        condition.clone(),
                    ],
                );
                if !deferred && (closed.contains(dst) || read(&values, dst)? != expected) {
                    bail!("history: qunload read version/window differs (§6.2)")
                }
                local.remove(dst);
                values.remove(dst);
                widths.remove(dst);
                types.remove(dst);
                dependencies.remove(dst);
                loads.remove(dst);
            }
            "qalloc" | "qcopy" => {
                if i.lhs.len() != 1 || args.len() != if op == "qalloc" { 2 } else { 1 } {
                    bail!("history: allocation arity (§3.3)")
                }
                let n = &i.lhs[0];
                if values.contains_key(n) {
                    bail!("history: duplicate live allocation (§3.3)")
                }
                let (mut v, ty) = if op == "qcopy" {
                    let src = name(&args[0])?;
                    let value = read(&values, src)?;
                    if !matches!(widths[src], Type::Wire(_)) {
                        bail!("history: qcopy requires bundle (§3.3)")
                    }
                    if quantum(&value) {
                        return Ok(false);
                    }
                    (value, widths[src].clone())
                } else {
                    name(&args[0])?;
                    let Operand::Int(w) = &args[1] else {
                        bail!("history: literal allocation width (§3.3)")
                    };
                    let w = w
                        .parse::<u32>()
                        .map_err(|_| Error("history: allocation width (§3.3)".into()))?;
                    if w == 0 {
                        bail!("history: zero width (§3.3)")
                    }
                    (zero(), Type::Wire(w))
                };
                if op == "qcopy" {
                    v = masked(v, &condition);
                }
                if op == "qcopy" {
                    let src = name(&args[0])?.to_string();
                    copies.insert(n.clone(), src.clone());
                    dependencies.insert(
                        n.clone(),
                        std::iter::once(src)
                            .chain(controls.iter().map(|(n, _)| n.clone()))
                            .collect(),
                    );
                }
                allocation_ids.insert(n.clone(), next_allocation);
                next_allocation = next_allocation
                    .checked_add(1)
                    .ok_or("host limit: symbolic allocation identity")?;
                values.insert(n.clone(), v);
                widths.insert(n.clone(), ty.clone());
                types.insert(n.clone(), ty);
                local.insert(n.clone());
            }
            "quncopy" | "qfree" => {
                if args.len() != if op == "qfree" { 1 } else { 2 } || !i.lhs.is_empty() {
                    bail!("history: release arity (§3.3)")
                }
                let n = name(args.last().unwrap())?;
                if op == "quncopy" && copies.get(n).map(String::as_str) != Some(name(&args[0])?) {
                    bail!("history: quncopy producer differs (§6.2)")
                }
                if op == "quncopy" && name(&args[0])? == n {
                    bail!("history: quncopy source/target overlap (§3.3)")
                }
                if !local.remove(n) {
                    bail!("history: release is not local (§3.3)")
                }
                if op == "quncopy" && widths.get(name(&args[0])?) != widths.get(n) {
                    bail!("history: quncopy width mismatch (§3.3)")
                }
                let expected = if op == "quncopy" {
                    masked(read(&values, name(&args[0])?)?, &condition)
                } else {
                    zero()
                };
                if !deferred && (closed.contains(n) || read(&values, n)? != expected) {
                    bail!(
                        "history v{VERSION}: exact computing sequence or read version differs (§6.2)"
                    )
                }
                copies.remove(n);
                loads.remove(n);
                dependencies.remove(n);
                values.remove(n);
                widths.remove(n);
                types.remove(n);
            }
            "oracle_call" | "action_call" => {
                // Reuse the operand checker without discarding the history.
                let mut cf = f.clone();
                cf.groups[0].extend(local.iter().map(|n| crate::object::Param {
                    name: n.clone(),
                    ty: widths[n].clone(),
                    role: Some("mut".into()),
                }));
                cf.blocks = vec![crate::object::Block {
                    name: "entry".into(),
                    params: vec![],
                    instrs: vec![i.clone()],
                }];
                crate::checker::static_parameter_action(o, &cf)?;
                let Some(Operand::Sym(callee)) = args.first() else {
                    bail!("history: callee (§3.3)")
                };
                for a in &args[if op == "oracle_call" { 2 } else { 1 }..if op == "oracle_call" {
                    6
                } else {
                    4
                }] {
                    let names = match a {
                        Operand::Name(n) => vec![n.clone()],
                        Operand::List(ns) => ns.clone(),
                        _ => vec![],
                    };
                    if names.iter().any(|n| controls.iter().any(|(c, _)| c == n)) {
                        bail!("history: effective control overlaps call operand (§3.3)")
                    }
                }
                let imported = o.function(callee).is_none();
                deferred |= imported;
                if op == "action_call" {
                    if imported {
                        continue;
                    }
                    let callee_fn = o.function(callee).unwrap();
                    if f.kind == crate::object::FnKind::PermAction
                        && callee_fn.kind != crate::object::FnKind::PermAction
                    {
                        bail!("history: nonpermutation callee (§5.3)")
                    }
                    let mut mutable = list(&args[1])?.to_vec();
                    for (actual, formal) in list(&args[3])?.iter().zip(&callee_fn.groups[2]) {
                        if formal.role.as_deref() == Some("mut") {
                            mutable.push(actual.clone());
                        }
                    }
                    let call_value = |key: String, values: Vec<Term>| {
                        if callee_fn.kind == crate::object::FnKind::PermAction {
                            Term::Permutation(key, values)
                        } else {
                            Term::Apply(key, values)
                        }
                    };

                    let mut context =
                        vec![Term::Atom(format!("args:{:?}", args[4])), condition.clone()];
                    for n in list(&args[2])?
                        .iter()
                        .chain(list(&args[3])?.iter().filter(|n| !mutable.contains(n)))
                    {
                        let value = read(&values, n)?;
                        if quantum(&value) && local.contains(n) {
                            closed.insert(n.clone());
                        }
                        context.push(value);
                    }
                    let invocation_locations = mutable
                        .iter()
                        .map(|n| (n, allocation_ids.get(n).copied()))
                        .collect::<Vec<_>>();
                    let key = format!(
                        "call:{callee}:{}:{invocation_locations:?}",
                        if adj { "adj" } else { "forward" }
                    );
                    let inverse = format!(
                        "call:{callee}:{}:{invocation_locations:?}",
                        if adj { "forward" } else { "adj" }
                    );
                    let prior = mutable
                        .iter()
                        .map(|n| read(&values, n))
                        .collect::<Result<Vec<_>>>()?;
                    if callee_fn.kind != crate::object::FnKind::PermAction {
                        for (copy, src) in &copies {
                            if mutable
                                .iter()
                                .chain(list(&args[2])?)
                                .chain(list(&args[3])?)
                                .any(|n| n == src)
                            {
                                closed.insert(copy.clone());
                            }
                        }
                    }
                    let candidate = prior.first().and_then(|t| {
                        if let Term::Apply(k, vs) | Term::Permutation(k, vs) = t {
                            if k == &format!("{inverse}:0") {
                                Some(vs.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    });
                    let restores = candidate.as_ref().is_some_and(|vs| {
                        vs.len() == mutable.len() + context.len()
                            && vs[mutable.len()..] == context
                            && prior.iter().enumerate().all(|(j, t)| {
                                *t == call_value(format!("{inverse}:{j}"), vs.clone())
                            })
                    });
                    let all = prior.iter().cloned().chain(context).collect::<Vec<_>>();
                    let touched = mutable
                        .iter()
                        .chain(list(&args[2])?)
                        .chain(list(&args[3])?)
                        .chain(controls.iter().map(|(n, _)| n))
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    let changed_regions = mutable
                        .iter()
                        .filter(|n| matches!(widths.get(*n), Some(Type::Region(..))))
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    {
                        let version_touches = if callee_fn.kind == crate::object::FnKind::PermAction
                        {
                            &changed_regions
                        } else {
                            &touched
                        };
                        for (w, deps) in &dependencies {
                            if !(restores && mutable.contains(w))
                                && values.get(w).is_some_and(|v| *v != zero())
                                && !deps.is_disjoint(version_touches)
                            {
                                closed.insert(w.clone());
                            }
                        }
                    }

                    for (j, n) in mutable.iter().enumerate() {
                        let old_deps = dependencies.get(n).cloned().unwrap_or_default();
                        let new = if restores {
                            candidate.as_ref().unwrap()[j].clone()
                        } else {
                            call_value(format!("{key}:{j}"), all.clone())
                        };
                        let mut deps = if restores {
                            producer_dependencies
                                .get(&prior[j])
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            let mut d = old_deps.clone();
                            d.extend(touched.iter().filter(|w| *w != n).cloned());
                            d
                        };
                        if !restores {
                            producer_dependencies.insert(new.clone(), old_deps);
                        }
                        if new == zero() {
                            deps.clear();
                        }
                        dependencies.insert(n.clone(), deps);
                        values.insert(n.clone(), new);
                    }
                    continue;
                }
                let token = name(&args[1])?;
                let mut inputs = vec![
                    Term::Atom(format!("token:{token}")),
                    Term::Atom(format!("args:{:?}", args[6])),
                    condition.clone(),
                ];
                for n in list(&args[2])?.iter().chain(list(&args[5])?) {
                    let value = read(&values, n)?;
                    if quantum(&value) && local.contains(n) {
                        closed.insert(n.clone());
                    }
                    inputs.push(value);
                }
                for (slot, a) in [("target", &args[3]), ("fault", &args[4])] {
                    let n = name(a)?;
                    if controls.iter().any(|(c, _)| c == n) {
                        bail!("history: effective control overlaps target (§3.3)")
                    }
                    let old = read(&values, n)?;
                    let term = Term::Oracle(format!("oracle:{callee}:{slot}"), inputs.clone());
                    let new = xor(old, term);
                    let deps = dependencies.entry(n.into()).or_default();
                    deps.extend(
                        list(&args[2])?
                            .iter()
                            .chain(list(&args[5])?)
                            .cloned()
                            .chain(controls.iter().map(|(n, _)| n.clone())),
                    );
                    if new == zero() {
                        deps.clear();
                    }
                    values.insert(n.into(), new);
                }
            }
            "qxor" => {
                if args.len() != 2 || !i.lhs.is_empty() {
                    bail!("history: qxor arity/result schema (§3.3)")
                }
                let (a, b) = (name(&args[0])?, name(&args[1])?);
                if a == b || widths.get(a) != widths.get(b) {
                    bail!("history: overlap/width (§3.3)")
                }
                if quantum(&read(&values, a)?) {
                    return Ok(false);
                }
                let new = xor(masked(read(&values, a)?, &condition), read(&values, b)?);
                let deps = dependencies.entry(b.into()).or_default();
                deps.insert(a.into());
                deps.extend(controls.iter().map(|(n, _)| n.clone()));
                if new == zero() {
                    deps.clear();
                }
                values.insert(b.into(), new);
            }
            _ => {
                if adj {
                    return Ok(false);
                }
                let rotation = matches!(op, "rx" | "ry" | "rz");
                if args.len() != if rotation { 2 } else { 1 } || !i.lhs.is_empty() {
                    bail!("history: gate arity (§3.3)")
                }
                let n = name(args.last().unwrap())?;
                if widths.get(n) != Some(&Type::Wire(1)) {
                    bail!("history: gate requires wire (§3.3)")
                }
                if controls.iter().any(|(c, _)| c == n) {
                    bail!("history: effective control overlaps target (§3.3)")
                }
                let old = read(&values, n)?;
                if f.kind == crate::object::FnKind::PermAction && op != "x" {
                    bail!("history: nonpermutation gate (§5.3)")
                }
                if matches!(op, "h" | "rx" | "ry") {
                    for (w, deps) in &dependencies {
                        if w != n && deps.contains(n) && values.get(w).is_some_and(|v| *v != zero())
                        {
                            closed.insert(w.clone());
                        }
                    }

                    for (copy, src) in &copies {
                        if src == n {
                            closed.insert(copy.clone());
                        }
                    }
                }
                if rotation {
                    match &args[0] {
                        Operand::Name(n) if types.get(n) == Some(&Type::F64) => {}
                        Operand::Float(v) | Operand::Int(v)
                            if v.parse::<f64>().is_ok_and(f64::is_finite) => {}
                        _ => bail!("history: angle type (§3.3)"),
                    }
                }
                let key = if rotation {
                    format!(
                        "{op}:{:?}:{condition:?}:wire:{n}:{}",
                        args[0],
                        allocation_ids.get(n).copied().unwrap_or(u64::MAX)
                    )
                } else {
                    format!(
                        "{op}:{condition:?}:wire:{n}:{}",
                        allocation_ids.get(n).copied().unwrap_or(u64::MAX)
                    )
                };
                let inverse = match op {
                    "sdg" => "s",
                    "s" => "sdg",
                    "tdg" => "t",
                    "t" => "tdg",
                    _ => op,
                };
                let inverse = format!(
                    "{inverse}:{condition:?}:wire:{n}:{}",
                    allocation_ids.get(n).copied().unwrap_or(u64::MAX)
                );
                let diagonal = matches!(op, "s" | "sdg" | "t" | "tdg" | "rz");
                // T5 phases on basis expressions do not alter their value;
                // phases on quantum histories must remain in that history.
                if diagonal && !quantum(&old) {
                    continue;
                }
                let old_deps = dependencies.get(n).cloned().unwrap_or_default();
                let restores = matches!(&old,Term::Apply(prior,vs)if !rotation && prior==&inverse && vs.len()==1);
                let new = if op == "x" {
                    xor(old.clone(), condition.clone())
                } else if let Term::Apply(prior, vs) = &old {
                    if !rotation && prior == &inverse && vs.len() == 1 {
                        vs[0].clone()
                    } else {
                        Term::Apply(key, vec![old.clone()])
                    }
                } else {
                    Term::Apply(key, vec![old.clone()])
                };
                let mut deps = if restores {
                    producer_dependencies.get(&old).cloned().unwrap_or_default()
                } else {
                    let mut d = old_deps.clone();
                    d.extend(controls.iter().map(|(n, _)| n.clone()));
                    d
                };
                if !restores && matches!(new, Term::Apply(..)) {
                    producer_dependencies.insert(new.clone(), old_deps);
                }
                if new == zero() {
                    deps.clear();
                }
                dependencies.insert(n.into(), deps);
                values.insert(n.into(), new);
            }
        }
    }
    if !local.is_empty() {
        bail!("history: live workspace at exit (§5.1/§6.2)")
    }
    Ok(true)
}
