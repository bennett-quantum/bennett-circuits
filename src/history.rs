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
    One,
    Atom(String),
    Apply(String, Vec<Term>),
    Permutation(String, Vec<Term>),
    Oracle(String, Vec<Term>),
    Xor(BTreeSet<Term>),
    Bits(Vec<Term>),
}
fn quantum(t: &Term) -> bool {
    match t {
        Term::Apply(..) => true,
        Term::Bits(ts) => ts.iter().any(quantum),
        Term::Xor(ts) => ts.iter().any(quantum),
        Term::Oracle(_, ts) | Term::Permutation(_, ts) => ts.iter().any(quantum),
        _ => false,
    }
}
fn contains(term: &Term, needle: &Term) -> bool {
    term == needle
        || match term {
            Term::Apply(_, ts)
            | Term::Permutation(_, ts)
            | Term::Oracle(_, ts)
            | Term::Bits(ts) => ts.iter().any(|t| contains(t, needle)),
            Term::Xor(ts) => ts.iter().any(|t| contains(t, needle)),
            Term::Atom(_) | Term::One => false,
        }
}
fn bit(value: &Term, i: usize) -> Term {
    match value {
        Term::Bits(bits) => bits[i].clone(),
        v if *v == zero() => zero(),
        _ => Term::Oracle(format!("bit:{i}"), vec![value.clone()]),
    }
}
fn bits(values: Vec<Term>) -> Term {
    if values.iter().all(|v| *v == zero()) {
        return zero();
    }
    if values.len() == 1 {
        return values[0].clone();
    }
    if let Some(Term::Oracle(key, original)) = values.first()
        && key == "bit:0"
        && original.len() == 1
        && values
            .iter()
            .enumerate()
            .all(|(i, v)| *v == bit(&original[0], i))
    {
        return original[0].clone();
    }
    Term::Bits(values)
}
fn wire_operand(o: &Operand) -> Result<String> {
    match o {
        Operand::Name(n) => Ok(n.clone()),
        Operand::Indexed(n, i) => Ok(format!("{n}[{i}]")),
        _ => bail!("history: expected wire (§3.3)"),
    }
}
fn location<'a>(n: &'a str, widths: &BTreeMap<String, Type>) -> Result<(&'a str, usize, usize)> {
    let (base, index) =
        if let Some((base, index)) = n.strip_suffix(']').and_then(|n| n.rsplit_once('[')) {
            (
                base,
                index
                    .parse::<usize>()
                    .map_err(|_| Error("history: wire index (§3.3)".into()))?,
            )
        } else {
            (n, 0)
        };
    let Some(Type::Wire(width)) = widths.get(base) else {
        bail!("history: live wire required (§3.3)")
    };
    if index >= *width as usize || (base == n && *width != 1) {
        bail!("history: scalar wire/index width (§3.3)")
    }
    Ok((base, index, *width as usize))
}
fn read_wire(
    values: &BTreeMap<String, Term>,
    widths: &BTreeMap<String, Type>,
    n: &str,
) -> Result<Term> {
    let (base, index, width) = location(n, widths)?;
    let value = values.get(base).ok_or("history: non-live wire (§3.3)")?;
    Ok(if width == 1 {
        value.clone()
    } else {
        bit(value, index)
    })
}
fn write_wire(
    values: &mut BTreeMap<String, Term>,
    widths: &BTreeMap<String, Type>,
    n: &str,
    value: Term,
) -> Result<()> {
    let (base, index, width) = location(n, widths)?;
    if width == 1 {
        values.insert(base.into(), value);
    } else {
        let original = values.get(base).ok_or("history: non-live wire")?;
        let mut parts = (0..width).map(|i| bit(original, i)).collect::<Vec<_>>();
        parts[index] = value;
        values.insert(base.into(), bits(parts));
    }
    Ok(())
}
fn reversible(key: &str, prior: &[Term], context: &[Term]) -> (Vec<Term>, bool) {
    let candidate = prior.first().and_then(|t| {
        if let Term::Permutation(k, vs) = t {
            (k == &format!("{key}:0")).then_some(vs)
        } else {
            None
        }
    });
    if let Some(vs) = candidate
        && vs.len() == prior.len() + context.len()
        && vs[prior.len()..] == *context
        && prior
            .iter()
            .enumerate()
            .all(|(i, v)| *v == Term::Permutation(format!("{key}:{i}"), vs.clone()))
    {
        return (vs[..prior.len()].to_vec(), true);
    }
    let values = prior.iter().chain(context).cloned().collect::<Vec<_>>();
    (
        (0..prior.len())
            .map(|i| Term::Permutation(format!("{key}:{i}"), values.clone()))
            .collect(),
        false,
    )
}
fn zero() -> Term {
    Term::Xor(BTreeSet::new())
}
fn xor(a: Term, b: Term) -> Term {
    let width = match (&a, &b) {
        (Term::Bits(v), _) | (_, Term::Bits(v)) => Some(v.len()),
        _ => None,
    };
    if let Some(width) = width {
        return bits((0..width).map(|i| xor(bit(&a, i), bit(&b, i))).collect());
    }
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
        if i.opcode == "qregion" {
            continue;
        } // Lexical declaration, including zero-trip bodies.
        if matches!(i.opcode.as_str(), "qpred" | "qunpred") {
            if i.operands.len() != 1 || !i.lhs.is_empty() {
                bail!("history: predicate schema (§3.3)")
            }
            let n = wire_operand(&i.operands[0])?;
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
            let n = wire_operand(&i.operands[0])?;
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
            if i.children.len() != 1 || !matches!(i.operands.len(), 1 | 3) || !i.lhs.is_empty() {
                bail!("history: qloop schema (§4.6)")
            }
            let Some(Operand::Int(n)) = i.operands.first() else {
                return Ok(false);
            };
            let n = n
                .parse::<u64>()
                .map_err(|_| Error("host limit: history loop count".into()))?;
            if n > 1_000_000 {
                bail!("host limit: materialising {n} symbolic loop iterations (§5.3)")
            }
            if i.operands.len() == 1 {
                for _ in 0..n {
                    if !prepare(&i.children[0], &active, out)? {
                        return Ok(false);
                    }
                }
                continue;
            }
            if !matches!(&i.operands[1],Operand::Name(k) if k == "while") || n == 0 {
                bail!("history: while schema (§4.6)")
            }
            let cond = name(&i.operands[2])?.to_string();
            let instr = |op: &str, lhs: Vec<String>, operands: Vec<Operand>| crate::object::Instr {
                opcode: op.into(),
                lhs,
                operands,
                children: vec![],
            };
            out.push((
                instr(
                    "history_while",
                    vec![],
                    vec![Operand::Name(cond.clone()), Operand::Int(n.to_string())],
                ),
                active.clone(),
            ));
            let prefix = format!("$history.while{}", out.len());
            let flags = (0..n).map(|j| format!("{prefix}.{j}")).collect::<Vec<_>>();
            for flag in &flags {
                out.push((
                    instr(
                        "qalloc",
                        vec![flag.clone()],
                        vec![Operand::Name("any".into()), Operand::Int("1".into())],
                    ),
                    active.clone(),
                ));
            }
            let compute = |j: usize| {
                let mut args = vec![];
                if j > 0 {
                    args.push(Operand::Name(flags[j - 1].clone()));
                }
                args.push(Operand::Indexed(cond.clone(), j as u32));
                args.push(Operand::Name(flags[j].clone()));
                instr(if j == 0 { "cx" } else { "ccx" }, vec![], args)
            };
            for (j, flag) in flags.iter().enumerate() {
                out.push((compute(j), active.clone()));
                let mut cs = active.clone();
                cs.push((flag.clone(), true));
                if !prepare(&i.children[0], &cs, out)? {
                    return Ok(false);
                }
            }
            for j in (0..flags.len()).rev() {
                out.push((compute(j), active.clone()));
            }
            for flag in flags {
                out.push((
                    instr("qfree", vec![], vec![Operand::Name(flag)]),
                    active.clone(),
                ));
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
                let n = wire_operand(plain.operands.first().ok_or("history: ctl operand")?)?;
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
fn conjunction(values: Vec<Term>) -> Term {
    if values.iter().any(|v| *v == zero()) {
        return zero();
    }
    let mut values = values
        .into_iter()
        .filter(|v| *v != Term::One)
        .collect::<Vec<_>>();
    values.sort();
    if values.is_empty() {
        Term::One
    } else if values.len() == 1 {
        values.pop().unwrap()
    } else {
        Term::Oracle("condition".into(), values)
    }
}
fn masked(value: Term, condition: &Term) -> Term {
    if value == zero() || *condition == zero() {
        zero()
    } else if *condition == Term::One {
        value
    } else {
        Term::Oracle("mask".into(), vec![condition.clone(), value])
    }
}
/// Returns false only when another universal checker must handle this body.
/// An unresolved body is deliberately pending, never a completed proof.
pub(crate) fn verify(o: &Object, f: &Function) -> Result<bool> {
    Ok(evaluate(o, f, &mut Evaluation::default())?.0)
}
#[derive(Default)]
struct Evaluation {
    active: BTreeSet<String>,
    completed: BTreeMap<String, (bool, bool)>,
}
fn evaluate(o: &Object, f: &Function, evaluation: &mut Evaluation) -> Result<(bool, bool)> {
    if evaluation.active.len() > 512 {
        bail!("host limit: symbolic call proof depth over 512")
    }
    if evaluation.active.contains(&f.name) {
        bail!("history: action call graph cycle (§5.1: acyclic)")
    }
    if let Some(result) = evaluation.completed.get(&f.name) {
        return Ok(*result);
    }
    evaluation.active.insert(f.name.clone());
    let result = evaluate_body(o, f, evaluation);
    evaluation.active.remove(&f.name);
    if let Ok(result) = &result {
        evaluation.completed.insert(f.name.clone(), *result);
    }
    result
}
/// Both flags are universal: a completed body proof and, independently,
/// equality of all interface values with their formal entry atoms. The latter
/// refines a permutation call without instantiating any token or argument.
fn evaluate_body(o: &Object, f: &Function, evaluation: &mut Evaluation) -> Result<(bool, bool)> {
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
        return Ok((false, false));
    }
    let instructions = prepared.iter().map(|(i, _)| i).collect::<Vec<_>>();
    let supported = [
        "qalloc",
        "qborrow",
        "qreturn",
        "cx",
        "ccx",
        "mcx",
        "swap",
        "cz",
        "qexch",
        "history_while",
        "qinplace",
        "qcopy",
        "qload",
        "qstore",
        "qexchm",
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
        return Ok((false, false));
    }
    let mut values = BTreeMap::<String, Term>::new();
    let mut widths = BTreeMap::new();
    let mut local = BTreeSet::new();
    let mut borrowed = BTreeMap::new();
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
    let mut local_regions = BTreeMap::new();
    let mut deferred = false;
    fn declarations<'a>(
        instructions: &'a [crate::object::Instr],
        out: &mut Vec<&'a crate::object::Instr>,
    ) {
        for instruction in instructions {
            out.push(instruction);
            for child in &instruction.children {
                declarations(child, out);
            }
        }
    }
    let mut lexical = vec![];
    declarations(&source, &mut lexical);
    let mut bounds = vec![];
    for instruction in lexical {
        if instruction.opcode == "history_while" {
            bail!("history: compiler-only annotation is not an ISA instruction (§4.8)")
        }
        if instruction.opcode == "qloop" {
            let Some(Operand::Int(n)) = instruction.operands.first() else {
                bail!("history: literal loop bound (§5.3)")
            };
            bounds.push(
                n.parse::<u64>()
                    .map_err(|_| Error("host limit: loop bound".into()))?,
            );
        }
        if instruction.opcode != "qregion" {
            continue;
        }
        if !instruction.lhs.is_empty()
            || !instruction.children.is_empty()
            || !matches!(instruction.operands.len(), 3 | 4)
        {
            bail!("history: qregion schema (§4.6)")
        }
        let region = name(&instruction.operands[0])?;
        let dimension = |i: usize| -> Result<u32> {
            let Operand::Int(n) = &instruction.operands[i] else {
                bail!("history: region dimension (§4.6)")
            };
            n.parse()
                .map_err(|_| Error("host limit: region dimension".into()))
        };
        let a = dimension(1)?;
        let w = dimension(2)?;
        if w == 0 || widths.contains_key(region) {
            bail!("history: local region width/identity (§4.6)")
        }
        if let Some(image) = instruction.operands.get(3) {
            let Operand::Sym(image) = image else {
                bail!("history: region image symbol (§4.6)")
            };
            let size = if let Some(data) = o.data_entry(image) {
                data.bytes.len() as u128
            } else if let Some(import) = o.imports.iter().find(|import| import.name == *image) {
                let signature = &import.signature;
                if signature.as_object().is_none_or(|s| s.len() != 4)
                    || signature.get("kind").and_then(crate::json::Value::as_str) != Some("data")
                    || signature.get("tier").and_then(crate::json::Value::as_str) != Some("-")
                    || signature.get("abi").and_then(crate::json::Value::as_str)
                        != Some(&o.header.object)
                {
                    bail!("history: imported region image signature (§9.1)")
                }
                deferred = true;
                u128::from(
                    signature
                        .get("size")
                        .and_then(crate::json::Value::as_u64)
                        .ok_or("history: data image size (§9.1)")?,
                )
            } else {
                bail!("history: missing region image (§4.6)")
            };
            if (1u128
                .checked_shl(a)
                .and_then(|n| n.checked_mul(u128::from(w))))
                != Some(size * 8)
            {
                bail!("history: region image size (§4.6)")
            }
        }
        let initial = Term::Atom(format!("local-image:{region}:{:?}", instruction.operands));
        values.insert(region.into(), initial.clone());
        local_regions.insert(region.to_string(), initial);
        widths.insert(region.into(), Type::Region(a, w));
        types.insert(region.into(), Type::Region(a, w));
    }
    if bounds != f.bounds {
        bail!("history: action bounds differ from text-order loops (§5.3)")
    }
    let mut copies = BTreeMap::<String, String>::new();
    let mut loads = BTreeMap::<String, (String, String)>::new();
    let mut dependencies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut producer_dependencies = BTreeMap::<Term, BTreeSet<String>>::new();
    let mut allocation_ids = BTreeMap::<String, u64>::new();
    let mut next_allocation = 0u64;
    let mut closed = BTreeSet::new();
    let mut phases = Vec::<Term>::new();
    let read = |vs: &BTreeMap<String, Term>, n: &str| -> Result<Term> {
        vs.get(n)
            .cloned()
            .ok_or_else(|| Error(format!("history: non-live bundle {n} (§3.3)")))
    };
    for (i, controls) in prepared {
        let mut cs = vec![];
        let mut control_locations = BTreeSet::new();
        for (n, positive) in &controls {
            let (base, index, _) = location(n, &widths)?;
            if !control_locations.insert((base.to_string(), index)) {
                bail!("history: duplicate effective wire (§3.3)")
            }
            let value = read_wire(&values, &widths, n)?;
            cs.push(if *positive {
                value
            } else {
                xor(value, Term::One)
            });
        }
        let condition = conjunction(cs);

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
            "history_while" => {
                let cond = name(&args[0])?;
                let Operand::Int(n) = &args[1] else {
                    bail!("history: while width")
                };
                if !matches!(widths.get(cond),Some(Type::Wire(w)) if w.to_string()==*n) {
                    bail!("history: while condition width/count (§4.6)")
                }
            }
            "cx" | "ccx" | "mcx" => {
                let count = if op == "ccx" { 3 } else { 2 };
                if args.len() != count || !i.lhs.is_empty() {
                    bail!("history: controlled X schema (§3.3)")
                }
                let target = wire_operand(&args[count - 1])?;
                let (base, index, _) = location(&target, &widths)?;
                let sources = if op == "mcx" {
                    list(&args[0])?.to_vec()
                } else {
                    args[..count - 1]
                        .iter()
                        .map(wire_operand)
                        .collect::<Result<Vec<_>>>()?
                };
                let mut touched = BTreeSet::new();
                touched.insert((base.to_string(), index));
                let mut predicates = vec![condition.clone()];
                let mut deps = BTreeSet::new();
                for source in sources {
                    let (n, i, _) = location(&source, &widths)?;
                    if !touched.insert((n.to_string(), i)) {
                        bail!("history: controlled X overlap (§3.3)")
                    }
                    predicates.push(read_wire(&values, &widths, &source)?);
                    deps.insert(n.to_string());
                }
                for (c, _) in &controls {
                    let (n, i, _) = location(c, &widths)?;
                    if !touched.insert((n.to_string(), i)) {
                        bail!("history: controlled X effective overlap (§3.3)")
                    }
                    deps.insert(n.to_string());
                }
                let new = xor(
                    read_wire(&values, &widths, &target)?,
                    conjunction(predicates),
                );
                dependencies.entry(base.into()).or_default().extend(deps);
                if new == zero() {
                    dependencies.remove(base);
                }
                write_wire(&mut values, &widths, &target, new)?;
            }
            "swap" | "qexch" => {
                if args.len() != 2 || !i.lhs.is_empty() {
                    bail!("history: exchange schema (§3.3)")
                }
                let mut names = args
                    .iter()
                    .map(|a| {
                        if op == "swap" {
                            wire_operand(a)
                        } else {
                            Ok(name(a)?.to_string())
                        }
                    })
                    .collect::<Result<Vec<_>>>()?;
                names.sort();
                let mut locations = vec![];
                for n in &names {
                    locations.push(if op == "swap" {
                        let (base, index, _) = location(n, &widths)?;
                        (base.to_string(), Some(index))
                    } else {
                        if !matches!(widths.get(n), Some(Type::Wire(_))) {
                            bail!("history: bundle exchange type (§3.3)")
                        };
                        (n.clone(), None)
                    });
                }
                if names[0] == names[1]
                    || (op == "swap" && locations[0] == locations[1])
                    || (op == "qexch" && widths.get(&names[0]) != widths.get(&names[1]))
                    || control_locations.iter().any(|(base, index)| {
                        locations
                            .iter()
                            .any(|(n, i)| n == base && i.is_none_or(|i| i == *index))
                    })
                {
                    bail!("history: exchange width/overlap (§3.3)")
                }
                let prior = names
                    .iter()
                    .map(|n| {
                        if op == "swap" {
                            read_wire(&values, &widths, n)
                        } else {
                            read(&values, n)
                        }
                    })
                    .collect::<Result<Vec<_>>>()?;
                let key = format!("{op}:{locations:?}:{controls:?}");
                let (next, restores) = if condition == Term::One {
                    (vec![prior[1].clone(), prior[0].clone()], false)
                } else {
                    reversible(&key, &prior, std::slice::from_ref(&condition))
                };
                let all_deps = locations
                    .iter()
                    .flat_map(|(n, _)| dependencies.get(n).into_iter().flatten().cloned())
                    .chain(control_locations.iter().map(|(n, _)| n.clone()))
                    .collect::<BTreeSet<_>>();
                for (j, n) in names.iter().enumerate() {
                    let base = &locations[j].0;
                    let old_deps = dependencies.get(base).cloned().unwrap_or_default();
                    let deps = if restores {
                        producer_dependencies
                            .get(&prior[j])
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        all_deps.clone()
                    };
                    if !restores {
                        producer_dependencies.insert(next[j].clone(), old_deps);
                    }
                    dependencies.insert(base.clone(), deps);
                    if op == "swap" {
                        write_wire(&mut values, &widths, n, next[j].clone())?;
                    } else {
                        values.insert(n.clone(), next[j].clone());
                    }
                }
            }
            "cz" => {
                if args.len() != 2
                    || !i.lhs.is_empty()
                    || f.kind == crate::object::FnKind::PermAction
                {
                    bail!("history: CZ schema/kind (§3.3; §5.3)")
                }
                let names = args.iter().map(wire_operand).collect::<Result<Vec<_>>>()?;
                let mut seen = control_locations.clone();
                let mut operands = vec![condition.clone()];
                for n in &names {
                    let (base, index, _) = location(n, &widths)?;
                    if !seen.insert((base.to_string(), index)) {
                        bail!("history: CZ overlap (§3.3)")
                    }
                    operands.push(read_wire(&values, &widths, n)?);
                }
                if seen.iter().any(|(base, _)| borrowed.contains_key(base)) {
                    bail!("history: non-permutation borrowed instruction (§5.6)")
                }
                if condition != zero() {
                    let phase = Term::Oracle("phase:cz".into(), operands);
                    if let Some(i) = phases.iter().rposition(|p| *p == phase) {
                        phases.remove(i);
                    } else {
                        phases.push(phase);
                    }
                }
            }
            "qreturn" => {
                if args.len() != 1 || !i.lhs.is_empty() {
                    bail!("history: qreturn schema (§3.3)")
                }
                let n = name(&args[0])?;
                let initial = borrowed
                    .remove(n)
                    .ok_or("history: qreturn producer (§5.6)")?;
                if !deferred
                    && (read(&values, n)? != initial
                        || closed.contains(n)
                        || values
                            .iter()
                            .any(|(other, value)| other != n && contains(value, &initial))
                        || phases.iter().any(|phase| contains(phase, &initial)))
                {
                    bail!("history: borrowed frame does not factor as identity (§5.6)")
                }
                local.remove(n);
                values.remove(n);
                widths.remove(n);
                types.remove(n);
                dependencies.remove(n);
            }
            "qexchm" => {
                if args.len() != 3 || !i.lhs.is_empty() {
                    bail!("history: qexchm schema (§3.3)")
                }
                let region = name(&args[0])?;
                let address = name(&args[1])?;
                let value = name(&args[2])?;
                let Some(Type::Region(a, w)) = widths.get(region) else {
                    bail!("history: exchange region (§3.3)")
                };
                if widths.get(address) != Some(&Type::Wire(*a))
                    || widths.get(value) != Some(&Type::Wire(*w))
                    || address == value
                    || control_locations
                        .iter()
                        .any(|(n, _)| n == address || n == value)
                {
                    bail!("history: memory exchange width/disjointness (§3.3)")
                }
                let prior = vec![read(&values, region)?, read(&values, value)?];
                let key = format!("exchange-memory:{region}:{address}:{value}:{controls:?}");
                let (next, _) = if condition == zero() {
                    (prior.clone(), true)
                } else {
                    reversible(&key, &prior, &[read(&values, address)?, condition.clone()])
                };
                for (workspace, deps) in &dependencies {
                    if deps.contains(region) {
                        closed.insert(workspace.clone());
                    }
                }
                values.insert(region.into(), next[0].clone());
                values.insert(value.into(), next[1].clone());
            }
            "qstore" => {
                if args.len() != 3 || !i.lhs.is_empty() {
                    bail!("history: qstore schema (§3.3)")
                }
                let region = name(&args[0])?;
                let address = name(&args[1])?;
                let value = name(&args[2])?;
                let Some(Type::Region(a, w)) = widths.get(region) else {
                    bail!("history: qstore region (§3.3)")
                };
                if widths.get(address) != Some(&Type::Wire(*a))
                    || widths.get(value) != Some(&Type::Wire(*w))
                    || address == value
                    || controls.iter().any(|(c, _)| {
                        location(c, &widths).is_ok_and(|(n, _, _)| n == address || n == value)
                    })
                {
                    bail!("history: qstore width/disjointness (§3.3)")
                }
                let context = vec![
                    read(&values, address)?,
                    read(&values, value)?,
                    condition.clone(),
                ];
                let prior = read(&values, region)?;
                let key = format!("store:{region}:{address}:{value}:{controls:?}");
                let new = if read(&values, value)? == zero() || condition == zero() {
                    prior.clone()
                } else {
                    match &prior {
                        Term::Permutation(k, vs)
                            if k == &key && vs.len() == context.len() + 1 && vs[1..] == context =>
                        {
                            vs[0].clone()
                        }
                        _ => {
                            Term::Permutation(key, std::iter::once(prior).chain(context).collect())
                        }
                    }
                };
                // A memory write advances cell versions even if an inverse
                // restores the value. Existing load windows stay closed.
                for (workspace, deps) in &dependencies {
                    if deps.contains(region) {
                        closed.insert(workspace.clone());
                    }
                }
                values.insert(region.into(), new);
            }
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
            "qalloc" | "qborrow" | "qcopy" => {
                if i.lhs.len() != 1 || args.len() != if op == "qcopy" { 1 } else { 2 } {
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
                    let initial = if op == "qborrow" {
                        Term::Atom(format!("borrow:{n}:{next_allocation}"))
                    } else {
                        zero()
                    };
                    if op == "qborrow" {
                        borrowed.insert(n.clone(), initial.clone());
                    }
                    (initial, Type::Wire(w))
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
                if op == "qfree"
                    && (borrowed.contains_key(n) || copies.contains_key(n) || loads.contains_key(n))
                {
                    bail!("history: qfree does not match allocation producer (§6.2)")
                }
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
            "qinplace" => {
                let mut cf = f.clone();
                cf.groups[0].extend(local.iter().map(|n| crate::object::Param {
                    name: n.clone(),
                    ty: widths[n].clone(),
                    role: Some("mut".into()),
                }));
                cf.groups[2].extend(local_regions.keys().map(|n| crate::object::Param {
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
                let n = name(&args[3])?;
                if controls.iter().any(|(c, _)| c == n) {
                    bail!("history: qinplace control overlap (§3.3)")
                }
                // §4.5 enforces the inverse-pair side condition at invocation,
                // before execution. This history proves cleanup for every valid
                // invocation; it does not manufacture that covering certificate.
                let key = format!(
                    "qinplace:{:?}:{:?}:{:?}:{condition:?}",
                    args[0], args[1], args[2]
                );
                let inverse = format!(
                    "qinplace:{:?}:{:?}:{:?}:{condition:?}",
                    args[1], args[0], args[2]
                );
                let old = read(&values, n)?;
                let new = match &old {
                    Term::Permutation(k, vs) if k == &inverse && vs.len() == 1 => vs[0].clone(),
                    _ => Term::Permutation(key, vec![old]),
                };
                values.insert(n.into(), new);
            }
            "oracle_call" | "action_call" => {
                // Reuse the operand checker without discarding the history.
                let mut cf = f.clone();
                cf.groups[0].extend(local.iter().map(|n| crate::object::Param {
                    name: n.clone(),
                    ty: widths[n].clone(),
                    role: Some("mut".into()),
                }));
                cf.groups[2].extend(local_regions.keys().map(|n| crate::object::Param {
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
                deferred |= imported
                    || o.function(callee)
                        .is_some_and(|cf| crate::checker::has_opaque_reference(o, cf));
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
                    // An identity proof is optional refinement of the exact
                    // call relation. If this complementary strategy cannot
                    // prove the callee body, retain the call history; the
                    // object's normal checker still validates every body.
                    let identity_body =
                        if callee_fn.kind == crate::object::FnKind::PermAction && !deferred {
                            match evaluate(o, callee_fn, evaluation) {
                                Ok((true, true)) => true,
                                Err(e) if e.0.contains("call graph cycle") => return Err(e),
                                _ => false,
                            }
                        } else {
                            false
                        };
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
                    if mutable.is_empty() && callee_fn.kind != crate::object::FnKind::PermAction {
                        let touched = list(&args[2])?
                            .iter()
                            .chain(list(&args[3])?)
                            .chain(controls.iter().map(|(n, _)| n))
                            .collect::<BTreeSet<_>>();
                        for (workspace, deps) in &dependencies {
                            if deps.iter().any(|n| touched.contains(n)) {
                                closed.insert(workspace.clone());
                            }
                        }
                        let phase = Term::Apply(format!("phase:{key}"), context.clone());
                        let inverse_phase =
                            Term::Apply(format!("phase:{inverse}"), context.clone());
                        if let Some(index) = phases.iter().rposition(|p| p == &inverse_phase) {
                            phases.remove(index);
                        } else {
                            phases.push(phase);
                        }
                        continue;
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
                    let no_outstanding_readers = prior.iter().all(|produced| {
                        values.iter().all(|(name, value)| {
                            mutable.contains(name) || !contains(value, produced)
                        }) && phases.iter().all(|phase| !contains(phase, produced))
                    });
                    let restores = (callee_fn.kind == crate::object::FnKind::PermAction
                        || no_outstanding_readers)
                        && candidate.as_ref().is_some_and(|vs| {
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

                    if identity_body {
                        continue;
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
                    return Ok((false, false));
                }
                let rotation = matches!(op, "rx" | "ry" | "rz");
                if args.len() != if rotation { 2 } else { 1 } || !i.lhs.is_empty() {
                    bail!("history: gate arity (§3.3)")
                }
                let wire = wire_operand(args.last().unwrap())?;
                let n = wire.as_str();
                let (base, index, width) = location(n, &widths)?;
                let canonical = if width == 1 {
                    base.to_string()
                } else {
                    format!("{base}[{index}]")
                };
                let n = canonical.as_str();
                if controls.iter().any(|(c, _)| {
                    location(c, &widths).is_ok_and(|(b, i, _)| b == base && i == index)
                }) {
                    bail!("history: effective control overlaps target (§3.3)")
                }
                if op != "x"
                    && (borrowed.contains_key(base)
                        || controls.iter().any(|(c, _)| {
                            location(c, &widths).is_ok_and(|(b, _, _)| borrowed.contains_key(b))
                        }))
                {
                    bail!("history: non-permutation instruction touches borrowed wires (§5.6)")
                }
                let old = read_wire(&values, &widths, n)?;
                if !matches!(op, "s" | "sdg" | "t" | "tdg" | "rz")
                    && quantum(&old)
                    && (values.iter().any(|(other, value)| {
                        if other != base {
                            contains(value, &old)
                        } else {
                            (0..width)
                                .filter(|i| *i != index)
                                .any(|i| contains(&bit(value, i), &old))
                        }
                    }) || phases.iter().any(|phase| contains(phase, &old)))
                {
                    closed.insert(base.into());
                }
                if f.kind == crate::object::FnKind::PermAction && op != "x" {
                    bail!("history: nonpermutation gate (§5.3)")
                }
                if matches!(op, "h" | "rx" | "ry") {
                    for (w, deps) in &dependencies {
                        if w != base
                            && deps.contains(base)
                            && values.get(w).is_some_and(|v| *v != zero())
                        {
                            closed.insert(w.clone());
                        }
                    }

                    for (copy, src) in &copies {
                        if src == base {
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
                let (angle_key, inverse_angle) = if rotation {
                    match &args[0] {
                        Operand::Name(name) => (format!("arg:{name}"), format!("neg-arg:{name}")),
                        Operand::Int(text) | Operand::Float(text) => {
                            let value = text
                                .parse::<f64>()
                                .map_err(|_| Error("history: angle (§3.3)".into()))?;
                            if value == 0. {
                                continue;
                            }
                            (
                                format!("bits:{:016x}", value.to_bits()),
                                format!("bits:{:016x}", (-value).to_bits()),
                            )
                        }
                        _ => bail!("history: angle (§3.3)"),
                    }
                } else {
                    (String::new(), String::new())
                };
                let inverse_op = match op {
                    "s" => "sdg",
                    "sdg" => "s",
                    "t" => "tdg",
                    "tdg" => "t",
                    _ => op,
                };
                let key = format!(
                    "{op}:{angle_key}:{condition:?}:wire:{n}:{}",
                    allocation_ids.get(base).copied().unwrap_or(u64::MAX)
                );
                let inverse = format!(
                    "{inverse_op}:{inverse_angle}:{condition:?}:wire:{n}:{}",
                    allocation_ids.get(base).copied().unwrap_or(u64::MAX)
                );
                let diagonal = matches!(op, "s" | "sdg" | "t" | "tdg" | "rz");
                // Retain diagonal phases separately from basis values. Exact
                // inverse phases cancel with multiplicity; a phase depending
                // on a borrowed system or an other producer cannot disappear
                // merely because that producer's bit expression is restored.
                if diagonal {
                    if condition == zero() || (op != "rz" && old == zero()) {
                        continue;
                    }
                    let phase = Term::Apply(
                        format!("phase:{op}:{angle_key}"),
                        vec![old.clone(), condition.clone()],
                    );
                    let inverse_phase = Term::Apply(
                        format!("phase:{inverse_op}:{inverse_angle}"),
                        vec![old.clone(), condition.clone()],
                    );
                    if let Some(index) = phases.iter().rposition(|p| p == &inverse_phase) {
                        phases.remove(index);
                    } else {
                        phases.push(phase);
                    }
                    continue;
                }
                let old_deps = dependencies.get(base).cloned().unwrap_or_default();
                let restores =
                    matches!(&old,Term::Apply(prior,vs)if prior==&inverse && vs.len()==1);
                let new = if op == "x" {
                    xor(old.clone(), condition.clone())
                } else if let Term::Apply(prior, vs) = &old {
                    if prior == &inverse && vs.len() == 1 {
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
                dependencies.insert(base.into(), deps);
                write_wire(&mut values, &widths, n, new)?;
            }
        }
    }
    if !deferred
        && local_regions
            .iter()
            .any(|(n, initial)| values.get(n) != Some(initial))
    {
        bail!("history: local region not restored to its image (§4.6; §6.2)")
    }
    if !local.is_empty() {
        bail!("history: live workspace at exit (§5.1/§6.2)")
    }
    let identity = !deferred
        && phases.is_empty()
        && f.groups[..3]
            .iter()
            .flatten()
            .all(|p| values.get(&p.name) == Some(&Term::Atom(p.name.clone())));
    Ok((true, identity))
}
