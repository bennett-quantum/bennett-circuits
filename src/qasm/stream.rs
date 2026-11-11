//! Compact flat oracle QASM. No whole-program IR or gate array is constructed.
use super::*;
use crate::{json::Value, object::Object, oracle_emit::Recipes};
use std::io::{BufRead, Write};
mod digest;
mod metadata;
mod reader;
use digest::Chain;
pub use reader::{StreamRead, visit_stream, visit_stream_checked};
/// Emit source-bound recipe QASM directly. Source checks/layout allocation finish
/// before the first byte. A completion record binds bytes, order and gate count.
pub fn write_oracle<W: Write + ?Sized>(
    source: &Object,
    recipes: &Recipes,
    writer: &mut W,
) -> Result<WriteStats> {
    let annotation = metadata::encode(source, recipes)?;
    metadata::decode(&annotation)?;
    let metadata = annotation.to_jcs();
    let wires = recipes.wire_count()?;
    let counts = recipes.counts();
    let digits = wires.max(1).ilog10() as usize + 1;
    let capacity = (counts.max_controls + 1)
        .checked_mul(digits + 18)
        .and_then(|n| n.checked_add(16))
        .ok_or("QASM: operation buffer overflow")?;
    let mut line = String::new();
    line.try_reserve(capacity)
        .map_err(|e| Error(format!("QASM: allocate operation buffer: {e}")))?;
    let mut digest = Chain::new(capacity.max(metadata.len() + 32))?;
    let mut output = crate::export::CountingWrite::new(writer);
    let mut stats = WriteStats {
        metadata_bytes: metadata.len(),
        peak_gate_objects: counts.peak_gate_objects + 1,
        resident_descriptors: counts.layer_descriptors,
        ..WriteStats::default()
    };
    for header in [
        "OPENQASM 3.0;\n".to_string(),
        "include \"stdgates.inc\";\n".into(),
        format!("// bc-stream: {metadata}\n"),
        format!("// interface: {}\n", recipes.interface_wires()),
        format!("qubit[{wires}] q;\n"),
    ] {
        emit(&mut output, &mut digest, &header)?;
    }
    recipes.visit(|gate| {
        crate::export::validate_gate(
            &FlatGate {
                gate: BaseGate::X,
                controls: gate.controls.clone(),
                targets: vec![gate.target],
            },
            wires,
        )?;
        line.clear();
        for (_, positive) in &gate.controls {
            line.push_str(if *positive { "ctrl @ " } else { "negctrl @ " });
        }
        line.push_str("x ");
        use std::fmt::Write as _;
        for (q, _) in &gate.controls {
            write!(&mut line, "q[{q}], ").map_err(|_| Error("QASM: format control".into()))?;
        }
        writeln!(&mut line, "q[{}];", gate.target)
            .map_err(|_| Error("QASM: format target".into()))?;
        emit(&mut output, &mut digest, &line)?;
        stats.writer_visits += 1;
        Ok(())
    })?;
    if stats.writer_visits != counts.full {
        bail!("QASM: recipe operation count changed");
    }
    let end = Value::Object(BTreeMap::from([
        ("scheme".into(), Value::Str("sha256-chain-v1".into())),
        ("digest".into(), Value::Str(digest.hex())),
        ("gates".into(), Value::Str(stats.writer_visits.to_string())),
    ]));
    put(&mut output, &format!("// bc-end: {}\n", end.to_jcs()))?;
    output.flush().map_err(io_error)?;
    stats.bytes = output.bytes;
    Ok(stats)
}
fn emit(writer: &mut dyn Write, digest: &mut Chain, line: &str) -> Result<()> {
    digest.feed(line.as_bytes())?;
    put(writer, line)
}

pub(super) fn parse_program(text: &str) -> Result<Program> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("// bc-stream: "))
        .ok_or("QASM: missing compact metadata")?;
    let (mut program, _, _, _) = metadata::decode(&Value::parse(value)?)?;
    let layout = super::local::Layout::new(&program.circuits[0])?;
    let mut ranges = layout
        .map
        .iter()
        .map(|(name, range)| (range.clone(), name.clone()))
        .collect::<Vec<_>>();
    ranges.sort_by_key(|(range, _)| (range.start, range.end));
    let qubit = |wire: usize| -> Result<QubitRef> {
        let position = ranges.partition_point(|(range, _)| range.end <= wire);
        let (range, name) = ranges
            .get(position)
            .filter(|(range, _)| range.contains(&wire))
            .ok_or("QASM: unmapped compact wire")?;
        Ok(QubitRef::new(
            name,
            (wire - range.start)
                .try_into()
                .map_err(|_| Error("QASM: compact bundle index".into()))?,
        ))
    };
    visit_stream_checked(std::io::Cursor::new(text), |gate| {
        program.circuits[0].body.push(Node::Gate(GateInst {
            gate: gate.gate,
            own_controls: gate.controls.len(),
            controls: gate
                .controls
                .iter()
                .map(|(q, p)| {
                    Ok(Ctrl {
                        qubit: qubit(*q)?,
                        positive: *p,
                    })
                })
                .collect::<Result<_>>()?,
            targets: gate
                .targets
                .iter()
                .map(|q| qubit(*q))
                .collect::<Result<_>>()?,
        }));
        Ok(())
    })?;
    Ok(program)
}

pub(super) fn materialize_flat(text: &str) -> Result<Flat> {
    let mut gates = vec![];
    let parsed = visit_stream(std::io::Cursor::new(text), |gate| {
        gates.push(gate.clone());
        Ok(())
    })?;
    Ok(Flat {
        num_qubits: parsed.qubits,
        interface: (0..parsed.interface).collect(),
        workspace: (parsed.interface..parsed.qubits).collect(),
        gates,
    })
}
