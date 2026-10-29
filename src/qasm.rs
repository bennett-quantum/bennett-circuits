//! OpenQASM 3 output: stdgates, hierarchical gates, ctrl/negctrl/inv.
//! Workspace is passed as explicit auxiliary gate operands. A versioned
//! comment preserves roles, geometry, and uncomputation pairs. `parse` accepts
//! our canonical subset and checks comments against executable text;
//! `parse_flat` independently reads executable text, ignoring IR metadata.
use crate::{
    Error, Result, bail,
    flatten::{Flat, FlatGate, flatten},
    ir::*,
    ir_codec,
};
use std::collections::{BTreeMap, BTreeSet};

fn one(c: &Circuit, body: Vec<Node>) -> Program {
    let mut c = c.clone();
    c.body = body;
    Program {
        top: c.name.clone(),
        circuits: vec![c],
    }
}
fn ident(i: usize) -> String {
    format!("bc{i}")
}
fn wire_map(c: &Circuit) -> BTreeMap<String, Vec<usize>> {
    let mut out = BTreeMap::new();
    let mut next = 0;
    for (n, w) in c.interface_bundles().into_iter().chain(
        c.workspace
            .iter()
            .map(|w| (w.name.clone(), u64::from(w.width))),
    ) {
        out.insert(n, (next..next + w as usize).collect());
        next += w as usize;
    }
    out
}
fn wr(map: &BTreeMap<String, Vec<usize>>, q: &QubitRef) -> Result<usize> {
    map.get(&q.bundle)
        .and_then(|x| x.get(q.index as usize))
        .copied()
        .ok_or_else(|| Error(format!("QASM: unknown wire {q:?}")))
}
fn invocation(name: &str, cs: &[(usize, bool)], args: &[usize], adj: bool) -> String {
    let mut s = String::from("  ");
    for (_, pos) in cs {
        s.push_str(if *pos { "ctrl @ " } else { "negctrl @ " });
    }
    if adj {
        s.push_str("inv @ ");
    }
    s.push_str(name);
    s.push(' ');
    s.push_str(
        &cs.iter()
            .map(|(q, _)| *q)
            .chain(args.iter().copied())
            .map(|q| format!("v{q}"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    s.push_str(";\n");
    s
}
fn order<'a>(
    p: &'a Program,
    c: &'a Circuit,
    active: &mut BTreeSet<String>,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<&'a Circuit>,
) -> Result<()> {
    if active.contains(&c.name) {
        bail!("QASM: call cycle at {}", c.name);
    }
    if seen.contains(&c.name) {
        return Ok(());
    }
    active.insert(c.name.clone());
    for n in &c.body {
        if let Node::Call { callee, .. } = n {
            order(
                p,
                p.circuit(callee)
                    .ok_or_else(|| Error(format!("QASM: unknown callee {callee}")))?,
                active,
                seen,
                out,
            )?;
        }
    }
    active.remove(&c.name);
    seen.insert(c.name.clone());
    out.push(c);
    Ok(())
}
pub fn export(p: &Program) -> Result<String> {
    let mut sorted = vec![];
    let mut seen = BTreeSet::new();
    for c in &p.circuits {
        order(p, c, &mut BTreeSet::new(), &mut seen, &mut sorted)?;
    }
    let mut text = format!(
        "OPENQASM 3.0;\ninclude \"stdgates.inc\";\n// bc: {}\n",
        ir_codec::encode(p).to_jcs()
    );
    let mut widths = BTreeMap::new();
    for c in sorted {
        let id = p.circuits.iter().position(|x| x.name == c.name).unwrap();
        let map = wire_map(c);
        let own = map.values().map(Vec::len).sum::<usize>();
        let mut next = own;
        let mut body = String::new();
        for node in &c.body {
            if let Node::Call {
                callee,
                adjoint,
                controls,
                args,
            } = node
            {
                let target = p.circuit(callee).unwrap();
                let cid = p.circuits.iter().position(|x| x.name == *callee).unwrap();
                let mut wires = vec![];
                for arg in args {
                    wires.extend(
                        map.get(arg)
                            .ok_or_else(|| Error(format!("QASM: unknown argument {arg}")))?,
                    );
                }
                if wires.len() != target.interface_wire_count() as usize {
                    bail!("QASM: call width mismatch");
                }
                let count = widths[callee] - wires.len();
                wires.extend(next..next + count);
                next += count;
                let cs = controls
                    .iter()
                    .map(|c| Ok((wr(&map, &c.qubit)?, c.positive)))
                    .collect::<Result<Vec<_>>>()?;
                body.push_str(&invocation(&ident(cid), &cs, &wires, *adjoint));
            } else {
                let flat = flatten(&one(c, vec![node.clone()]))?;
                for g in flat.gates {
                    body.push_str(&invocation(
                        &ir_codec::gate(g.gate),
                        &g.controls,
                        &g.targets,
                        false,
                    ));
                }
            }
        }
        // OpenQASM gate definitions require at least one qubit. An inert
        // sentinel handles a zero-wire identity, without changing the IR.
        let width = next.max(1);
        widths.insert(c.name.clone(), width);
        text.push_str(&format!(
            "gate {} {} {{\n{body}}}\n",
            ident(id),
            (0..width)
                .map(|i| format!("v{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let top = p.top_circuit()?;
    let width = widths[&p.top];
    let id = p.circuits.iter().position(|c| c.name == p.top).unwrap();
    text.push_str(&format!(
        "qubit[{width}] q;\n{} {};\n",
        ident(id),
        (0..width)
            .map(|i| format!("q[{i}]"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    // Interface metadata is intentionally separate from the IR codec:
    // the executable parser never decodes the hierarchical IR.
    text.push_str(&format!("// interface: {}\n", top.interface_wire_count()));
    Ok(text)
}
pub fn parse(text: &str) -> Result<Program> {
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("// bc: "))
        .ok_or_else(|| Error("QASM: missing IR metadata".into()))?;
    let p = ir_codec::decode(&crate::json::Value::parse(line)?)?;
    if export(&p)? != text {
        bail!("QASM: executable text and metadata differ (or noncanonical subset)");
    }
    parse_flat(text)?;
    Ok(p)
}
#[derive(Clone)]
struct Op {
    name: String,
    controls: Vec<bool>,
    inverse: bool,
    wires: Vec<usize>,
}
struct Def {
    width: usize,
    ops: Vec<Op>,
}
fn index(s: &str, top: bool) -> Result<usize> {
    let s = if top {
        s.strip_prefix("q[").and_then(|s| s.strip_suffix(']'))
    } else {
        s.strip_prefix('v')
    };
    s.ok_or_else(|| Error("QASM: malformed qubit operand".into()))?
        .parse()
        .map_err(|_| Error("QASM: invalid wire index".into()))
}
fn op(line: &str, top: bool) -> Result<Op> {
    let mut line = line
        .trim()
        .strip_suffix(';')
        .ok_or_else(|| Error("QASM: missing semicolon".into()))?;
    let mut controls = vec![];
    let mut inverse = false;
    while let Some((modifier, rest)) = line.split_once(" @ ") {
        match modifier {
            "ctrl" => controls.push(true),
            "negctrl" => controls.push(false),
            "inv" => inverse = !inverse,
            _ => bail!("QASM: unknown modifier {modifier}"),
        };
        line = rest;
    }
    let (name, args) = line
        .split_once(' ')
        .ok_or_else(|| Error("QASM: missing operands".into()))?;
    let wires = args
        .split(", ")
        .map(|s| index(s, top))
        .collect::<Result<_>>()?;
    Ok(Op {
        name: name.into(),
        controls,
        inverse,
        wires,
    })
}
fn expand(
    op: &Op,
    defs: &BTreeMap<String, Def>,
    map: &[usize],
    outer: &[(usize, bool)],
    inverse: bool,
    out: &mut Vec<FlatGate>,
    active: &mut BTreeSet<String>,
) -> Result<()> {
    let wires = op
        .wires
        .iter()
        .map(|i| {
            map.get(*i)
                .copied()
                .ok_or_else(|| Error("QASM: wire index outside declaration".into()))
        })
        .collect::<Result<Vec<_>>>()?;
    if wires.len() < op.controls.len() {
        bail!("QASM: too few control operands");
    }
    let mut cs = outer.to_vec();
    cs.extend(wires.iter().copied().zip(op.controls.iter().copied()));
    let targets = &wires[op.controls.len()..];
    let adj = inverse ^ op.inverse;
    if let Some(def) = defs.get(&op.name) {
        if targets.len() != def.width {
            bail!("QASM: call arity mismatch");
        }
        if !active.insert(op.name.clone()) {
            bail!("QASM: recursive gate definition");
        }
        let ops: Box<dyn Iterator<Item = &Op>> = if adj {
            Box::new(def.ops.iter().rev())
        } else {
            Box::new(def.ops.iter())
        };
        for child in ops {
            expand(child, defs, targets, &cs, adj, out, active)?;
        }
        active.remove(&op.name);
    } else {
        let mut gate = ir_codec::read_gate(&op.name)?;
        if adj {
            gate = gate.adjoint();
        }
        if targets.len() != gate.arity() {
            bail!("QASM: gate arity mismatch");
        }
        let mut seen = BTreeSet::new();
        for w in cs.iter().map(|(q, _)| q).chain(targets) {
            if !seen.insert(w) {
                bail!("QASM: overlapping gate operands");
            }
        }
        out.push(FlatGate {
            gate,
            controls: cs,
            targets: targets.to_vec(),
        });
    }
    Ok(())
}
/// Parse actual OpenQASM statements, ignoring the `bc` IR comments.
pub fn parse_flat(text: &str) -> Result<Flat> {
    let mut lines = text.lines();
    if lines.next() != Some("OPENQASM 3.0;") || lines.next() != Some("include \"stdgates.inc\";") {
        bail!("QASM: unsupported header");
    }
    let mut defs = BTreeMap::new();
    let mut current: Option<(String, Def)> = None;
    let mut width = None;
    let mut root = None;
    let mut interface = None;
    for line in lines {
        if let Some(n) = line.strip_prefix("// interface: ") {
            interface = Some(
                n.parse::<usize>()
                    .map_err(|_| Error("QASM: invalid interface count".into()))?,
            );
            continue;
        }
        if line.starts_with("//") {
            continue;
        }
        if let Some(header) = line.strip_prefix("gate ") {
            if current.is_some() {
                bail!("QASM: nested definition");
            }
            let header = header
                .strip_suffix(" {")
                .ok_or_else(|| Error("QASM: malformed definition".into()))?;
            let (name, args) = header
                .split_once(' ')
                .ok_or_else(|| Error("QASM: missing formals".into()))?;
            let args = args.split(", ").collect::<Vec<_>>();
            for (i, a) in args.iter().enumerate() {
                if index(a, false)? != i {
                    bail!("QASM: noncanonical formals");
                }
            }
            current = Some((
                name.into(),
                Def {
                    width: args.len(),
                    ops: vec![],
                },
            ));
        } else if line == "}" {
            let (name, def) = current
                .take()
                .ok_or_else(|| Error("QASM: unexpected }".into()))?;
            if defs.insert(name, def).is_some() {
                bail!("QASM: duplicate gate");
            }
        } else if let Some(n) = line
            .strip_prefix("qubit[")
            .and_then(|s| s.strip_suffix("] q;"))
        {
            if width.is_some() || current.is_some() {
                bail!("QASM: unexpected qubit declaration");
            }
            width = Some(
                n.parse::<usize>()
                    .map_err(|_| Error("QASM: invalid qubit count".into()))?,
            );
        } else if let Some((_, def)) = &mut current {
            def.ops.push(op(line, false)?);
        } else {
            if root.is_some() {
                bail!("QASM: multiple root calls");
            }
            root = Some(op(line, true)?);
        }
    }
    if current.is_some() {
        bail!("QASM: unclosed definition");
    }
    let width = width.ok_or_else(|| Error("QASM: no qubit declaration".into()))?;
    let interface = interface.ok_or_else(|| Error("QASM: no interface count".into()))?;
    if interface > width {
        bail!("QASM: interface exceeds allocation");
    }
    let mut gates = vec![];
    expand(
        &root.ok_or_else(|| Error("QASM: no root call".into()))?,
        &defs,
        &(0..width).collect::<Vec<_>>(),
        &[],
        false,
        &mut gates,
        &mut BTreeSet::new(),
    )?;
    Ok(Flat {
        num_qubits: width,
        interface: (0..interface).collect(),
        workspace: (interface..width).collect(),
        gates,
    })
}
