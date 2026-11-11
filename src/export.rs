//! Shared wire-layout transport and validation for executable exporters.
use crate::{
    Result, bail,
    flatten::{Flat, FlatGate, flatten},
    ir::{BaseGate, Program},
    ir_codec,
    json::Value,
};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) fn validate_gate(g: &FlatGate, n: usize) -> Result<()> {
    if g.targets.len() != g.gate.arity() {
        bail!("export: gate target arity")
    }
    if matches!(g.gate, BaseGate::Rx(x)|BaseGate::Ry(x)|BaseGate::Rz(x) if !x.is_finite()) {
        bail!("export: non-finite angle")
    }
    let mut seen = BTreeSet::new();
    for q in g.targets.iter().chain(g.controls.iter().map(|(q, _)| q)) {
        if *q >= n || !seen.insert(*q) {
            bail!("export: wire outside layout or overlapping operands")
        }
    }
    Ok(())
}
pub(crate) fn prepare(p: &Program) -> Result<Flat> {
    let flat = flatten(p)?;
    for gate in &flat.gates {
        validate_gate(gate, flat.num_qubits)?;
    }
    Ok(flat)
}
pub(crate) fn metadata(p: &Program, f: &Flat, comment: &str) -> String {
    let indices = |v: &[usize]| Value::Array(v.iter().map(|n| Value::Int(n.to_string())).collect());
    let layout = Value::Object(BTreeMap::from([
        ("qubits".into(), Value::Int(f.num_qubits.to_string())),
        ("interface".into(), indices(&f.interface)),
        ("workspace".into(), indices(&f.workspace)),
    ]));
    format!(
        "{comment} bc: {}\n{comment} bc-layout: {}\n",
        ir_codec::encode(p).to_jcs(),
        layout.to_jcs()
    )
}
pub(crate) fn layout(text: &str, comment: &str) -> Result<Flat> {
    let prefix = format!("{comment} bc-layout: ");
    let mut records = text.lines().filter_map(|l| l.strip_prefix(&prefix));
    let v = Value::parse(records.next().ok_or("export parser: missing wire layout")?)?;
    if records.next().is_some() {
        bail!("export parser: duplicate layout")
    }
    let m = v.as_object().ok_or("export parser: layout record")?;
    if m.len() != 3 {
        bail!("export parser: layout fields")
    }
    let number = |v: &Value| -> Result<usize> {
        usize::try_from(v.as_u64().ok_or("export parser: unsigned wire index")?)
            .map_err(|_| crate::Error("host limit: wire index".into()))
    };
    let n = number(m.get("qubits").ok_or("export parser: qubits")?)?;
    let indices = |key: &str| -> Result<Vec<usize>> {
        m.get(key)
            .and_then(Value::as_array)
            .ok_or("export parser: wire list")?
            .iter()
            .map(number)
            .collect()
    };
    let interface = indices("interface")?;
    let workspace = indices("workspace")?;
    let mut seen = BTreeSet::new();
    for q in interface.iter().chain(&workspace) {
        if *q >= n || !seen.insert(*q) {
            bail!("export parser: wire partition")
        }
    }
    if seen.len() != n {
        bail!("export parser: incomplete wire partition")
    }
    Ok(Flat {
        num_qubits: n,
        interface,
        workspace,
        gates: vec![],
    })
}
pub(crate) fn source(text: &str, comment: &str) -> Result<Program> {
    let prefix = format!("{comment} bc: ");
    let mut records = text.lines().filter_map(|l| l.strip_prefix(&prefix));
    let p = ir_codec::decode(&Value::parse(
        records.next().ok_or("export parser: missing source IR")?,
    )?)?;
    if records.next().is_some() {
        bail!("export parser: duplicate source IR")
    }
    Ok(p)
}

/// Counts actual accepted bytes, including partial writes, without buffering.
pub(crate) struct CountingWrite<'a, W: std::io::Write + ?Sized> {
    inner: &'a mut W,
    pub bytes: u128,
}
impl<'a, W: std::io::Write + ?Sized> CountingWrite<'a, W> {
    pub fn new(inner: &'a mut W) -> Self {
        Self { inner, bytes: 0 }
    }
}
impl<W: std::io::Write + ?Sized> std::io::Write for CountingWrite<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self.inner.write(bytes)?;
        self.bytes += count as u128;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
/// Complete a file beside its destination, then atomically replace it. An error
/// leaves any prior destination intact and removes the incomplete temporary.
pub fn write_atomic(
    path: &std::path::Path,
    emit: impl FnOnce(&mut dyn std::io::Write) -> Result<()>,
) -> Result<()> {
    use std::{
        fs::OpenOptions,
        io::{BufWriter, Write},
        sync::atomic::{AtomicU64, Ordering},
    };
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let name = path.file_name().ok_or("export: missing output filename")?;
    let (mut file, temp) = loop {
        let mut name = name.to_os_string();
        name.push(format!(
            ".partial-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let temp = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (file, PartialFile(temp)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(crate::Error(format!("export: create output: {e}"))),
        }
    };
    {
        let mut writer = BufWriter::new(&mut file);
        emit(&mut writer)?;
        writer
            .flush()
            .map_err(|e| crate::Error(format!("export: flush output: {e}")))?;
    }
    file.sync_all()
        .map_err(|e| crate::Error(format!("export: sync output: {e}")))?;
    std::fs::rename(&temp.0, path)
        .map_err(|e| crate::Error(format!("export: complete output: {e}")))?;
    Ok(())
}
struct PartialFile(std::path::PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
