//! QIR 1.0 with static qubit handles and the QIR runtime/QIS controlled ABI.
//! General controls use runtime arrays; this is unrestricted QIR, not Base
//! Profile. Rotations use the legacy `{ double, Qubit* }` controlled tuple.
//! No angle decomposition, rounding, measurement or phase erasure occurs.
use crate::{
    Error, Result, bail, export,
    flatten::{Flat, FlatGate},
    ir::{BaseGate, Program},
};
use std::collections::{BTreeMap, BTreeSet};
fn qubit(q: usize) -> String {
    format!("%Qubit* inttoptr (i64 {q} to %Qubit*)")
}
struct Writer {
    body: String,
    declarations: BTreeSet<String>,
    next: usize,
}
impl Writer {
    fn single(
        &mut self,
        name: &str,
        adj: bool,
        angle: Option<f64>,
        controls: &[(usize, bool)],
        target: usize,
    ) {
        for (q, pos) in controls {
            if !pos {
                self.single("x", false, None, &[], *q)
            }
        }
        let id = self.next;
        self.next += 1;
        let controlled = !controls.is_empty();
        if controlled {
            self.declarations
                .insert("declare %Array* @__quantum__rt__array_create_1d(i32, i64)".into());
            self.declarations.insert(
                "declare i8* @__quantum__rt__array_get_element_ptr_1d(%Array*, i64)".into(),
            );
            self.declarations.insert(
                "declare void @__quantum__rt__array_update_reference_count(%Array*, i32)".into(),
            );
            self.body.push_str(&format!(
                "  %c{id} = call %Array* @__quantum__rt__array_create_1d(i32 8, i64 {})\n",
                controls.len()
            ));
            for (i, (q, _)) in controls.iter().enumerate() {
                self.body.push_str(&format!("  %e{id}_{i} = call i8* @__quantum__rt__array_get_element_ptr_1d(%Array* %c{id}, i64 {i})\n  %p{id}_{i} = bitcast i8* %e{id}_{i} to %Qubit**\n  store {}, %Qubit** %p{id}_{i}\n",qubit(*q)));
            }
        }
        let suffix = match (controlled, adj) {
            (false, false) => "body",
            (false, true) => "adj",
            (true, false) => "ctl",
            (true, true) => "ctladj",
        };
        let symbol = format!("__quantum__qis__{name}__{suffix}");
        let (types, args) = match (angle, controlled) {
            (None, false) => ("%Qubit*".to_string(), qubit(target)),
            (None, true) => (
                "%Array*, %Qubit*".to_string(),
                format!("%Array* %c{id}, {}", qubit(target)),
            ),
            (Some(a), false) => (
                "double, %Qubit*".to_string(),
                format!("double 0x{:016X}, {}", a.to_bits(), qubit(target)),
            ),
            (Some(a), true) => {
                self.body.push_str(&format!("  %r{id} = alloca {{ double, %Qubit* }}\n  store {{ double, %Qubit* }} {{ double 0x{:016X}, {} }}, {{ double, %Qubit* }}* %r{id}\n  %t{id} = bitcast {{ double, %Qubit* }}* %r{id} to i8*\n",a.to_bits(),qubit(target)));
                (
                    "%Array*, i8*".to_string(),
                    format!("%Array* %c{id}, i8* %t{id}"),
                )
            }
        };
        self.declarations
            .insert(format!("declare void @{symbol}({types})"));
        self.body
            .push_str(&format!("  call void @{symbol}({args})\n"));
        if controlled {
            self.body.push_str(&format!(
                "  call void @__quantum__rt__array_update_reference_count(%Array* %c{id}, i32 -1)\n"
            ));
        }
        for (q, pos) in controls.iter().rev() {
            if !pos {
                self.single("x", false, None, &[], *q)
            }
        }
    }
    fn gate(&mut self, g: &FlatGate) {
        let t = g.targets[0];
        match g.gate {
            BaseGate::Swap => {
                let b = g.targets[1];
                for (a, b) in [(t, b), (b, t), (t, b)] {
                    let mut cs = g.controls.clone();
                    cs.push((a, true));
                    self.single("x", false, None, &cs, b);
                }
            }
            BaseGate::Cz => {
                let mut cs = g.controls.clone();
                cs.push((t, true));
                self.single("z", false, None, &cs, g.targets[1]);
            }
            _ => {
                let (name, adj, angle) = match g.gate {
                    BaseGate::X => ("x", false, None),
                    BaseGate::H => ("h", false, None),
                    BaseGate::S => ("s", false, None),
                    BaseGate::Sdg => ("s", true, None),
                    BaseGate::T => ("t", false, None),
                    BaseGate::Tdg => ("t", true, None),
                    BaseGate::Rx(a) => ("rx", false, Some(a)),
                    BaseGate::Ry(a) => ("ry", false, Some(a)),
                    BaseGate::Rz(a) => ("rz", false, Some(a)),
                    _ => unreachable!(),
                };
                self.single(name, adj, angle, &g.controls, t);
            }
        }
    }
}
pub fn export(p: &Program) -> Result<String> {
    let f = export::prepare(p)?;
    let mut writer = Writer {
        body: String::new(),
        declarations: BTreeSet::new(),
        next: 0,
    };
    for g in &f.gates {
        writer.gate(g)
    }
    let mut out = format!(
        "; Bennett circuit; QIR 1.0 unrestricted\n{}target datalayout = \"e-p:64:64\"\n%Qubit = type opaque\n%Array = type opaque\n\ndefine void @main() #0 {{\nentry:\n{}  ret void\n}}\n\n",
        export::metadata(p, &f, ";"),
        writer.body
    );
    for declaration in writer.declarations {
        out.push_str(&declaration);
        out.push('\n');
    }
    out.push_str(&format!("\nattributes #0 = {{ \"entry_point\" \"required_num_qubits\"=\"{}\" \"required_num_results\"=\"0\" }}\n!llvm.module.flags = !{{!0, !1, !2, !3}}\n!0 = !{{i32 1, !\"qir_major_version\", i32 1}}\n!1 = !{{i32 7, !\"qir_minor_version\", i32 0}}\n!2 = !{{i32 1, !\"dynamic_qubit_management\", i1 false}}\n!3 = !{{i32 1, !\"dynamic_result_management\", i1 false}}\n",f.num_qubits));
    Ok(out)
}
pub fn parse(text: &str) -> Result<Program> {
    let p = export::source(text, ";")?;
    if export(&p)? != text {
        bail!("QIR: executable text and metadata differ (or noncanonical subset)")
    }
    parse_flat(text)?;
    Ok(p)
}
fn index(s: &str) -> Result<usize> {
    let s = s
        .strip_prefix("%Qubit* inttoptr (i64 ")
        .and_then(|s| s.strip_suffix(" to %Qubit*)"))
        .ok_or("QIR: static qubit operand")?;
    s.parse().map_err(|_| Error("QIR: qubit index".into()))
}
fn angle(s: &str) -> Result<f64> {
    let s = s
        .strip_prefix("double 0x")
        .ok_or("QIR: exact binary64 literal")?;
    if s.len() != 16 {
        bail!("QIR: angle bit width")
    }
    let n = u64::from_str_radix(s, 16).map_err(|_| Error("QIR: angle bits".into()))?;
    let f = f64::from_bits(n);
    if !f.is_finite() {
        bail!("QIR: non-finite angle")
    };
    Ok(f)
}
/// A small interpreter for the emitted LLVM/QIR subset. It reads actual QIS
/// calls and their runtime-array stores, and never decodes the source IR.
pub fn parse_flat(text: &str) -> Result<Flat> {
    let mut f = export::layout(text, ";")?;
    let mut arrays = BTreeMap::<String, Vec<Option<usize>>>::new();
    let mut elements = BTreeMap::<String, (String, usize)>::new();
    let mut tuples = BTreeMap::<String, Option<(f64, usize)>>::new();
    let mut tuple_pointers = BTreeMap::<String, String>::new();
    let mut defined = BTreeSet::new();
    let mut inside = false;
    let mut ended = false;
    let mut returned = false;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with(';') {
            continue;
        }
        if l == "define void @main() #0 {" {
            if inside || ended {
                bail!("QIR: duplicate entry point")
            };
            inside = true;
            continue;
        }
        if !inside {
            if l.starts_with("declare ")
                || l.starts_with("target datalayout = ")
                || l == "%Qubit = type opaque"
                || l == "%Array = type opaque"
                || l.starts_with("attributes #0 = ")
                || l.starts_with('!')
            {
                continue;
            }
            bail!("QIR: unknown module statement: {l}")
        }
        if l == "entry:" {
            continue;
        }
        if l == "}" {
            if !returned {
                bail!("QIR: missing return")
            };
            inside = false;
            ended = true;
            continue;
        }
        if returned {
            bail!("QIR: instruction after return")
        }
        if l == "ret void" {
            returned = true;
            continue;
        }
        if let Some((lhs, rhs)) = l.split_once(" = ") {
            if !lhs.starts_with('%') || !defined.insert(lhs.to_string()) {
                bail!("QIR: duplicate/malformed SSA value")
            }
            if let Some(n) = rhs
                .strip_prefix("call %Array* @__quantum__rt__array_create_1d(i32 8, i64 ")
                .and_then(|s| s.strip_suffix(')'))
            {
                let n = n
                    .parse::<usize>()
                    .map_err(|_| Error("QIR: array length".into()))?;
                if n == 0 || n > f.num_qubits {
                    bail!("QIR: control array length")
                }
                arrays.insert(lhs.into(), vec![None; n]);
                continue;
            }
            if let Some(args) = rhs
                .strip_prefix("call i8* @__quantum__rt__array_get_element_ptr_1d(%Array* ")
                .and_then(|s| s.strip_suffix(')'))
            {
                let (array, n) = args
                    .split_once(", i64 ")
                    .ok_or("QIR: array index operands")?;
                let n = n
                    .parse::<usize>()
                    .map_err(|_| Error("QIR: array index".into()))?;
                if arrays.get(array).and_then(|a| a.get(n)).is_none() {
                    bail!("QIR: array access outside allocation")
                }
                elements.insert(lhs.into(), (array.into(), n));
                continue;
            }
            if let Some(element) = rhs
                .strip_prefix("bitcast i8* ")
                .and_then(|s| s.strip_suffix(" to %Qubit**"))
            {
                let binding = elements
                    .get(element)
                    .ok_or("QIR: undefined array element")?
                    .clone();
                elements.insert(lhs.into(), binding);
                continue;
            }
            if rhs == "alloca { double, %Qubit* }" {
                tuples.insert(lhs.into(), None);
                continue;
            }
            if let Some(tuple) = rhs
                .strip_prefix("bitcast { double, %Qubit* }* ")
                .and_then(|s| s.strip_suffix(" to i8*"))
            {
                if !tuples.contains_key(tuple) {
                    bail!("QIR: undefined argument tuple")
                }
                tuple_pointers.insert(lhs.into(), tuple.into());
                continue;
            }
            bail!("QIR: unknown SSA instruction: {rhs}")
        }
        if let Some(store) = l.strip_prefix("store %Qubit* ") {
            let (q, pointer) = store.rsplit_once(", %Qubit** ").ok_or("QIR: qubit store")?;
            let q = index(&format!("%Qubit* {q}"))?;
            if q >= f.num_qubits {
                bail!("QIR: stored qubit outside layout")
            }
            let (array, n) = elements
                .get(pointer)
                .ok_or("QIR: undefined store pointer")?;
            let slot = &mut arrays.get_mut(array).ok_or("QIR: released array store")?[*n];
            if slot.replace(q).is_some() {
                bail!("QIR: duplicate control-array store")
            };
            continue;
        }
        if let Some(store) = l.strip_prefix("store { double, %Qubit* } { ") {
            let (value, pointer) = store
                .split_once(" }, { double, %Qubit* }* ")
                .ok_or("QIR: rotation tuple store")?;
            let (a, q) = value.split_once(", ").ok_or("QIR: rotation tuple fields")?;
            let slot = tuples
                .get_mut(pointer)
                .ok_or("QIR: undefined tuple store")?;
            if slot.replace((angle(a)?, index(q)?)).is_some() {
                bail!("QIR: duplicate tuple store")
            };
            continue;
        }
        if let Some(array) = l
            .strip_prefix("call void @__quantum__rt__array_update_reference_count(%Array* ")
            .and_then(|s| s.strip_suffix(", i32 -1)"))
        {
            if arrays.remove(array).is_none() {
                bail!("QIR: release of unknown array")
            };
            continue;
        }
        if let Some(call) = l.strip_prefix("call void @__quantum__qis__") {
            let (symbol, args) = call.split_once('(').ok_or("QIR: call syntax")?;
            let args = args.strip_suffix(')').ok_or("QIR: call end")?;
            let (name, suffix) = symbol.split_once("__").ok_or("QIR: intrinsic suffix")?;
            let controlled = matches!(suffix, "ctl" | "ctladj");
            let adj = matches!(suffix, "adj" | "ctladj");
            if !matches!(suffix, "body" | "adj" | "ctl" | "ctladj") {
                bail!("QIR: intrinsic variant")
            }
            let mut controls = vec![];
            let operand = if controlled {
                let (a, rest) = args
                    .split_once(", ")
                    .ok_or("QIR: controlled call operands")?;
                let a = a
                    .strip_prefix("%Array* ")
                    .ok_or("QIR: control array operand")?;
                controls = arrays
                    .get(a)
                    .ok_or("QIR: unknown control array")?
                    .iter()
                    .map(|q| Ok((q.ok_or("QIR: uninitialised control array")?, true)))
                    .collect::<Result<Vec<_>>>()?;
                rest
            } else {
                args
            };
            let rotation = matches!(name, "rx" | "ry" | "rz");
            let (a, target) = if rotation && controlled {
                let ptr = operand
                    .strip_prefix("i8* ")
                    .ok_or("QIR: controlled rotation tuple")?;
                let tuple = tuple_pointers.get(ptr).ok_or("QIR: undefined tuple cast")?;
                tuples
                    .get(tuple)
                    .and_then(|v| *v)
                    .ok_or("QIR: uninitialised rotation tuple")?
            } else if rotation {
                let (a, q) = operand.split_once(", ").ok_or("QIR: rotation operands")?;
                (angle(a)?, index(q)?)
            } else {
                (0., index(operand)?)
            };
            let mut gate = match name {
                "x" => BaseGate::X,
                "h" => BaseGate::H,
                "s" => BaseGate::S,
                "t" => BaseGate::T,
                "rx" => BaseGate::Rx(a),
                "ry" => BaseGate::Ry(a),
                "rz" => BaseGate::Rz(a),
                "z" => BaseGate::Cz,
                _ => bail!("QIR: unknown QIS intrinsic {name}"),
            };
            if adj {
                gate = gate.adjoint()
            }
            let targets = if name == "z" {
                let (q, _) = controls
                    .pop()
                    .ok_or("QIR: Z requires a control in the exported subset")?;
                vec![q, target]
            } else {
                vec![target]
            };
            let g = FlatGate {
                gate,
                controls,
                targets,
            };
            export::validate_gate(&g, f.num_qubits)?;
            f.gates.push(g);
            continue;
        }
        bail!("QIR: unknown executable instruction: {l}")
    }
    if inside || !ended || !arrays.is_empty() {
        bail!("QIR: incomplete function or unreleased control array")
    }
    Ok(f)
}
