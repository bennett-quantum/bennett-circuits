//! OpenQASM 3 output: stdgates, hierarchical gates, ctrl/negctrl/inv.
//! Workspace is passed as explicit auxiliary gate operands. A versioned
//! comment preserves roles, geometry, and uncomputation pairs. `parse` accepts
//! our canonical subset and checks comments against executable text;
//! `parse_flat` independently reads executable text, ignoring IR metadata.
use crate::{
    Error, Result, bail,
    flatten::{Flat, FlatGate},
    ir::*,
    ir_codec,
};
use std::collections::{BTreeMap, BTreeSet};

fn ident(i: usize) -> String {
    format!("bc{i}")
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
mod legacy;
mod local;
mod stream;
pub use crate::export::write_atomic;
pub use legacy::write;
pub use stream::{StreamRead, visit_stream, visit_stream_checked, write_oracle};

#[derive(Clone, Debug, Default)]
pub struct WriteStats {
    pub writer_visits: u128,
    pub bytes: u128,
    pub metadata_bytes: usize,
    pub peak_gate_objects: usize,
    pub resident_descriptors: usize,
    pub circuit_body_clones: usize,
    pub per_gate_program_scans: usize,
}
fn io_error(error: std::io::Error) -> Error {
    Error(format!("QASM I/O: {error}"))
}
fn put(writer: &mut dyn std::io::Write, text: &str) -> Result<()> {
    writer.write_all(text.as_bytes()).map_err(io_error)
}
/// Small-caller adapter to the indexed writer. Legacy metadata remains lossless.
pub fn export(p: &Program) -> Result<String> {
    let mut bytes = vec![];
    write(p, &mut bytes)?;
    String::from_utf8(bytes).map_err(|_| Error("QASM writer emitted invalid UTF-8".into()))
}
pub fn parse(text: &str) -> Result<Program> {
    if text.lines().any(|line| line.starts_with("// bc-stream: ")) {
        return stream::parse_program(text);
    }
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
    if !text.lines().any(|line| line.starts_with("gate ")) {
        return stream::materialize_flat(text);
    }
    parse_hierarchical_flat(text)
}
fn parse_hierarchical_flat(text: &str) -> Result<Flat> {
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

/// Placed executable leaves with attached source hierarchy, proof obligations,
/// class assignments, logical mapping, timing and target binding.
pub fn export_placed(p: &crate::placement::Placed) -> Result<String> {
    export_placed_annotations(p, p.annotations()?)
}
/// Explicit ISA 0.9-draft §9.4 frame metadata on placed executable output.
pub fn export_placed_draft(
    p: &crate::placement::Placed,
    o: &crate::object::Object,
) -> Result<String> {
    export_placed_annotations(p, p.annotations_draft(o)?)
}
fn export_placed_annotations(
    p: &crate::placement::Placed,
    annotations: crate::json::Value,
) -> Result<String> {
    let n = p.flat.num_qubits.max(1);
    let mut text = format!(
        "OPENQASM 3.0;\ninclude \"stdgates.inc\";\n// bc-placement: {}\n// interface: {}\ngate placed {} {{\n",
        annotations.to_jcs(),
        p.flat.interface.len(),
        (0..n)
            .map(|q| format!("v{q}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for g in &p.flat.gates {
        text.push_str(&invocation(
            &crate::ir_codec::gate(g.gate),
            &g.controls,
            &g.targets,
            false,
        ));
    }
    text.push_str(&format!(
        "}}\nqubit[{n}] q;\nplaced {};\n",
        (0..n)
            .map(|q| format!("q[{q}]"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    Ok(text)
}
