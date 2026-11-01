//! Quil executable export; CONTROLLED/DAGGER follow the Quil specification.
//! Hierarchy, allocation kinds and source proof material travel in comments.
use crate::{
    Result, bail, export,
    flatten::{Flat, FlatGate},
    ir::{BaseGate, Program},
};
fn spelling(g: BaseGate) -> String {
    match g {
        BaseGate::X => "X".into(),
        BaseGate::H => "H".into(),
        BaseGate::S => "S".into(),
        BaseGate::Sdg => "DAGGER S".into(),
        BaseGate::T => "T".into(),
        BaseGate::Tdg => "DAGGER T".into(),
        BaseGate::Cz => "CZ".into(),
        BaseGate::Swap => "SWAP".into(),
        BaseGate::Rx(a) => format!("RX({a:?})"),
        BaseGate::Ry(a) => format!("RY({a:?})"),
        BaseGate::Rz(a) => format!("RZ({a:?})"),
    }
}
pub fn export(p: &Program) -> Result<String> {
    let f = export::prepare(p)?;
    let mut out = format!("# Bennett circuit; Quil\n{}", export::metadata(p, &f, "#"));
    for g in &f.gates {
        for (q, positive) in &g.controls {
            if !positive {
                out.push_str(&format!("X {q}\n"));
            }
        }
        out.push_str(&"CONTROLLED ".repeat(g.controls.len()));
        out.push_str(&spelling(g.gate));
        for q in g.controls.iter().map(|(q, _)| q).chain(&g.targets) {
            out.push_str(&format!(" {q}"));
        }
        out.push('\n');
        for (q, positive) in g.controls.iter().rev() {
            if !positive {
                out.push_str(&format!("X {q}\n"));
            }
        }
    }
    out.push_str("HALT\n");
    Ok(out)
}
pub fn parse(text: &str) -> Result<Program> {
    let p = export::source(text, "#")?;
    if export(&p)? != text {
        bail!("Quil: executable text and metadata differ (or noncanonical subset)")
    }
    parse_flat(text)?;
    Ok(p)
}
/// Parse executable instructions without reading the source IR comment.
/// The independent wire-layout comment supplies the interface/clean partition.
pub fn parse_flat(text: &str) -> Result<Flat> {
    let mut f = export::layout(text, "#")?;
    let mut halted = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if halted {
            bail!("Quil: instruction after HALT")
        }
        if line == "HALT" {
            halted = true;
            continue;
        }
        let mut words = line.split_whitespace();
        let mut controls = 0;
        let mut adj = false;
        let mut op = words.next().ok_or("Quil: instruction")?;
        while matches!(op, "CONTROLLED" | "DAGGER") {
            if op == "CONTROLLED" {
                controls += 1
            } else {
                adj = !adj
            }
            op = words.next().ok_or("Quil: missing gate after modifier")?;
        }
        let mut gate = match op {
            "X" => BaseGate::X,
            "H" => BaseGate::H,
            "S" => BaseGate::S,
            "T" => BaseGate::T,
            "CZ" => BaseGate::Cz,
            "SWAP" => BaseGate::Swap,
            _ => {
                let (name, arg) = op.split_once('(').ok_or("Quil: unknown gate")?;
                let angle = arg
                    .strip_suffix(')')
                    .ok_or("Quil: angle syntax")?
                    .parse::<f64>()
                    .map_err(|_| crate::Error("Quil: angle".into()))?;
                match name {
                    "RX" => BaseGate::Rx(angle),
                    "RY" => BaseGate::Ry(angle),
                    "RZ" => BaseGate::Rz(angle),
                    _ => bail!("Quil: unknown rotation"),
                }
            }
        };
        if adj {
            gate = gate.adjoint()
        }
        let qs = words
            .map(|s| {
                s.parse::<usize>()
                    .map_err(|_| crate::Error("Quil: qubit index".into()))
            })
            .collect::<Result<Vec<_>>>()?;
        if qs.len() != controls + gate.arity() {
            bail!("Quil: operand arity")
        }
        let g = FlatGate {
            gate,
            controls: qs[..controls].iter().map(|q| (*q, true)).collect(),
            targets: qs[controls..].to_vec(),
        };
        export::validate_gate(&g, f.num_qubits)?;
        f.gates.push(g);
    }
    if !halted {
        bail!("Quil: missing HALT")
    }
    Ok(f)
}
