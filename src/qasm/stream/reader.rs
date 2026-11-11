//! Independent direct-statement parser; visitor sees one actual gate at a time.
use super::*;
#[derive(Clone, Debug)]
pub struct StreamRead {
    pub qubits: usize,
    pub interface: usize,
    pub gates: u128,
    pub bytes: u128,
    pub metadata: Option<Value>,
    pub peak_gate_objects: usize,
    pub histogram: BTreeMap<usize, u128>,
}
/// Read executable direct-statement QASM, ignoring source/IR/completion comments.
/// Legacy hierarchy remains supported by parse_flat; this streaming subset has
/// no gate definitions to retain or expand into another gate array.
pub fn visit_stream(
    reader: impl BufRead,
    visitor: impl FnMut(&FlatGate) -> Result<()>,
) -> Result<StreamRead> {
    read(reader, false, visitor)
}
/// Also verify compact version/layout/counts and the mandatory completion hash.
/// A visitor must discard its result on Err: transport errors can occur at EOF.
pub fn visit_stream_checked(
    reader: impl BufRead,
    visitor: impl FnMut(&FlatGate) -> Result<()>,
) -> Result<StreamRead> {
    read(reader, true, visitor)
}
fn read(
    mut input: impl BufRead,
    checked: bool,
    mut visitor: impl FnMut(&FlatGate) -> Result<()>,
) -> Result<StreamRead> {
    let mut line = vec![];
    let mut hash = Chain::new(1024)?;
    let mut state = State {
        width: None,
        interface: None,
        metadata: None,
        end: false,
        gates: 0,
        bytes: 0,
        lines: 0,
        histogram: BTreeMap::new(),
    };
    while next_line(&mut input, &mut line)? {
        state.bytes += line.len() as u128;
        state.lines += 1;
        let text = std::str::from_utf8(&line).map_err(|_| Error("QASM: invalid UTF-8".into()))?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        if state.lines <= 2 {
            let expected = if state.lines == 1 {
                "OPENQASM 3.0;"
            } else {
                "include \"stdgates.inc\";"
            };
            if text != expected {
                bail!("QASM: unsupported stream header");
            }
        } else if checked && text.starts_with("// bc-end: ") {
            state.finish(text, &hash)?;
            continue;
        } else {
            state.line(text, checked, &mut visitor)?;
        }
        if checked {
            hash.feed(&line)?;
        }
    }
    state.result(checked)
}
struct State {
    width: Option<usize>,
    interface: Option<usize>,
    metadata: Option<Value>,
    end: bool,
    gates: u128,
    bytes: u128,
    lines: usize,
    histogram: BTreeMap<usize, u128>,
}
impl State {
    fn line(
        &mut self,
        text: &str,
        checked: bool,
        visitor: &mut impl FnMut(&FlatGate) -> Result<()>,
    ) -> Result<()> {
        if self.end {
            bail!("QASM: data after completion record");
        }
        if let Some(value) = text.strip_prefix("// interface: ") {
            if self.interface.is_some() || self.width.is_some() {
                bail!("QASM: duplicate/misplaced interface count");
            }
            self.interface = Some(
                value
                    .parse()
                    .map_err(|_| Error("QASM: invalid interface count".into()))?,
            );
            return Ok(());
        }
        if checked && let Some(value) = text.strip_prefix("// bc-stream: ") {
            if self.metadata.is_some() || self.width.is_some() {
                bail!("QASM: duplicate/misplaced stream metadata");
            }
            let metadata = Value::parse(value)?;
            metadata::decode(&metadata)?;
            self.metadata = Some(metadata);
            return Ok(());
        }
        if text.starts_with("//") {
            return Ok(());
        }
        if let Some(value) = text
            .strip_prefix("qubit[")
            .and_then(|s| s.strip_suffix("] q;"))
        {
            if self.width.is_some() {
                bail!("QASM: duplicate declaration");
            }
            let width = value
                .parse::<usize>()
                .map_err(|_| Error("QASM: invalid qubit count".into()))?;
            if width == 0 || self.interface.ok_or("QASM: missing interface count")? > width {
                bail!("QASM: invalid wire layout");
            }
            if checked {
                let metadata = self
                    .metadata
                    .as_ref()
                    .ok_or("QASM: missing stream metadata")?;
                let (_, n, i, _) = metadata::decode(metadata)?;
                if n != width || Some(i) != self.interface {
                    bail!("QASM: annotation/declaration mismatch");
                }
            }
            self.width = Some(width);
            return Ok(());
        }
        let width = self.width.ok_or("QASM: operation before declaration")?;
        let parsed = super::super::op(text, true)?;

        let kind = ir_codec::read_gate(&parsed.name)?;
        let gate = FlatGate {
            gate: if parsed.inverse { kind.adjoint() } else { kind },
            controls: parsed
                .wires
                .iter()
                .copied()
                .zip(parsed.controls.iter().copied())
                .collect(),
            targets: parsed
                .wires
                .get(parsed.controls.len()..)
                .ok_or("QASM: too few control operands")?
                .to_vec(),
        };
        crate::export::validate_gate(&gate, width)?;
        visitor(&gate)?;
        self.gates += 1;
        *self.histogram.entry(gate.controls.len()).or_insert(0) += 1;
        Ok(())
    }
    fn finish(&mut self, text: &str, hash: &Chain) -> Result<()> {
        if self.end || self.width.is_none() {
            bail!("QASM: misplaced completion");
        }
        let value = Value::parse(text.strip_prefix("// bc-end: ").unwrap())?;
        metadata::keys(&value, &["scheme", "digest", "gates"])?;
        if metadata::field(&value, "scheme")?.as_str() != Some("sha256-chain-v1")
            || metadata::field(&value, "digest")?.as_str() != Some(hash.hex().as_str())
            || metadata::number(metadata::field(&value, "gates")?)? != self.gates
        {
            bail!("QASM: completion digest/count mismatch");
        }
        self.end = true;
        Ok(())
    }
    fn result(self, checked: bool) -> Result<StreamRead> {
        let (qubits, interface) = (
            self.width.ok_or("QASM: missing declaration")?,
            self.interface.ok_or("QASM: missing interface count")?,
        );
        if self.lines < 3 {
            bail!("QASM: incomplete stream");
        }
        if checked {
            if !self.end {
                bail!("QASM: missing completion record");
            }
            let (_, _, _, expected) = metadata::decode(
                self.metadata
                    .as_ref()
                    .ok_or("QASM: missing stream metadata")?,
            )?;
            if expected != self.gates {
                bail!("QASM: plan/executable count mismatch");
            }
            let hist = metadata::field(
                metadata::field(self.metadata.as_ref().unwrap(), "counts")?,
                "histogram",
            )?
            .as_object()
            .unwrap();
            let expected = hist
                .iter()
                .map(|(k, v)| {
                    Ok((
                        k.parse::<usize>()
                            .map_err(|_| Error("QASM: histogram arity".into()))?,
                        metadata::number(v)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?
                .into_iter()
                .filter(|(_, v)| *v > 0)
                .collect::<BTreeMap<_, _>>();
            if self.histogram != expected {
                bail!("QASM: plan/executable histogram mismatch");
            }
        }
        Ok(StreamRead {
            qubits,
            interface,
            gates: self.gates,
            bytes: self.bytes,
            metadata: self.metadata,
            peak_gate_objects: usize::from(self.gates > 0),
            histogram: self.histogram,
        })
    }
}
fn next_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> Result<bool> {
    line.clear();
    loop {
        let available = reader.fill_buf().map_err(io_error)?;
        if available.is_empty() {
            return Ok(!line.is_empty());
        }
        let take = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        let end = available[take - 1] == b'\n';
        line.try_reserve(take)
            .map_err(|e| Error(format!("QASM: allocate input line: {e}")))?;
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if end {
            return Ok(true);
        }
    }
}
