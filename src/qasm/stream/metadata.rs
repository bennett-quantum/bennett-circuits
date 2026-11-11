use super::*;
use crate::{
    json::Value as V,
    oracle::{Argument, Budget},
};
fn string(value: impl ToString) -> V {
    V::Str(value.to_string())
}
fn object(fields: impl IntoIterator<Item = (&'static str, V)>) -> V {
    V::Object(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn budget(b: Budget) -> V {
    V::Array(
        [b.steps, b.heap, b.stack, b.depth]
            .into_iter()
            .map(string)
            .collect(),
    )
}
fn argument(a: &Argument) -> V {
    match a {
        Argument::Integer(s) => V::Array(vec![string("integer"), string(s)]),
        Argument::Float(f) => V::Array(vec![
            string("binary64"),
            string(format!("{:016x}", f.to_bits())),
        ]),
        Argument::Token(b) => V::Array(vec![string("token"), budget(*b)]),
    }
}
pub(super) fn encode(
    obj: &crate::object::Object,
    recipes: &crate::oracle_emit::Recipes,
) -> Result<V> {
    let report = obj.check()?;
    let binding = recipes.plan().binding();
    if report.object_hash != binding.object_hash
        || report
            .symbols
            .get(&binding.oracle)
            .and_then(|s| s.closure_hash.as_ref())
            != Some(&binding.closure_hash)
    {
        bail!("QASM: recipe/source identity mismatch");
    }
    let layout = recipes.program_layout()?;
    let counts = recipes.counts();
    Ok(object([
        ("version", string("bc-oracle-stream-1")),
        (
            "source_evidence",
            string("source-identity-only; no target-realization certificate"),
        ),
        (
            "binding",
            object([
                ("object_hash", string(&binding.object_hash)),
                ("closure_hash", string(&binding.closure_hash)),
                ("oracle", string(&binding.oracle)),
                ("budget", budget(binding.budget)),
                (
                    "arguments",
                    V::Array(binding.arguments.iter().map(argument).collect()),
                ),
            ]),
        ),
        ("layout", ir_codec::encode(&layout)),
        (
            "counts",
            object([
                ("forward", string(counts.forward)),
                ("commit", string(counts.commit)),
                ("inverse", string(counts.forward)),
                ("total", string(counts.full)),
                ("qubits", string(counts.qubits)),
                ("history", string(counts.history_wires)),
                ("peak_recipe_gates", string(counts.peak_gate_objects)),
                ("descriptors", string(counts.layer_descriptors)),
                ("analysis_visits", string(recipes.plan().analysis_visits)),
                ("max_controls", string(counts.max_controls)),
                (
                    "histogram",
                    V::Object(
                        counts
                            .histogram
                            .iter()
                            .map(|(k, v)| (k.to_string(), string(v)))
                            .collect(),
                    ),
                ),
            ]),
        ),
    ]))
}
pub(super) fn decode(value: &V) -> Result<(Program, usize, usize, u128)> {
    validate_binding(value)?;
    let binding = field(value, "binding")?;
    let p = ir_codec::decode(field(value, "layout")?)?;
    if p.circuits.len() != 1
        || p.top
            != field(binding, "oracle")?
                .as_str()
                .ok_or("QASM: source region")?
        || !p.circuits[0].body.is_empty()
        || !p.circuits[0].mem.is_empty()
    {
        bail!("QASM: invalid compact program layout");
    }
    let c = &p.circuits[0];
    let layout = super::super::local::Layout::new(c)?;
    let interface = c.interface_wire_count() as usize;
    let counts = field(value, "counts")?;
    keys(
        counts,
        &[
            "forward",
            "commit",
            "inverse",
            "total",
            "qubits",
            "history",
            "peak_recipe_gates",
            "descriptors",
            "analysis_visits",
            "max_controls",
            "histogram",
        ],
    )?;
    for name in [
        "forward",
        "commit",
        "inverse",
        "total",
        "qubits",
        "history",
        "peak_recipe_gates",
        "descriptors",
        "analysis_visits",
        "max_controls",
    ] {
        number(field(counts, name)?)?;
    }
    let total = number(field(counts, "total")?)?;
    let forward = number(field(counts, "forward")?)?;
    let commit = number(field(counts, "commit")?)?;
    if forward.checked_mul(2).and_then(|n| n.checked_add(commit)) != Some(total)
        || number(field(counts, "inverse")?)? != forward
    {
        bail!("QASM: inconsistent stream counts");
    }
    if number(field(counts, "qubits")?)? != layout.own as u128 {
        bail!("QASM: layout/count mismatch");
    }
    let history = c
        .workspace
        .iter()
        .filter(|w| w.kind == WsKind::RetainedHistory)
        .map(|w| u128::from(w.width))
        .sum::<u128>();
    if number(field(counts, "history")?)? != history {
        bail!("QASM: history/count mismatch");
    }
    validate_histogram(counts, total)?;
    Ok((p, layout.own, interface, total))
}
fn validate_binding(value: &V) -> Result<()> {
    keys(
        value,
        &["version", "source_evidence", "binding", "layout", "counts"],
    )?;
    if field(value, "version")?.as_str() != Some("bc-oracle-stream-1")
        || field(value, "source_evidence")?.as_str()
            != Some("source-identity-only; no target-realization certificate")
    {
        bail!("QASM: unsupported stream metadata");
    }
    let binding = field(value, "binding")?;
    keys(
        binding,
        &[
            "object_hash",
            "closure_hash",
            "oracle",
            "budget",
            "arguments",
        ],
    )?;
    for name in ["object_hash", "closure_hash"] {
        let hash = field(binding, name)?.as_str().ok_or("QASM: source hash")?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("QASM: invalid source hash");
        }
    }
    let limits = field(binding, "budget")?
        .as_array()
        .ok_or("QASM: budget array")?;
    if limits.len() != 4 {
        bail!("QASM: budget arity");
    }
    for limit in limits {
        u64::try_from(number(limit)?).map_err(|_| Error("QASM: budget exceeds u64".into()))?;
    }
    validate_arguments(field(binding, "arguments")?)?;
    Ok(())
}
fn validate_arguments(value: &V) -> Result<()> {
    for value in value.as_array().ok_or("QASM: argument list")? {
        let pair = value.as_array().ok_or("QASM: typed argument")?;
        if pair.len() != 2 {
            bail!("QASM: argument arity");
        }
        match pair[0].as_str() {
            Some("integer") => {
                let s = pair[1].as_str().ok_or("QASM: integer argument")?;
                let digits = s.strip_prefix('-').unwrap_or(s);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    bail!("QASM: integer argument");
                }
            }
            Some("binary64") => {
                let s = pair[1].as_str().ok_or("QASM: binary64 argument")?;
                let bits = u64::from_str_radix(s, 16)
                    .map_err(|_| Error("QASM: binary64 argument".into()))?;
                if s.len() != 16 || !f64::from_bits(bits).is_finite() {
                    bail!("QASM: nonfinite binary64 argument");
                }
            }
            Some("token") => {
                let bs = pair[1].as_array().ok_or("QASM: token argument")?;
                if bs.len() != 4 {
                    bail!("QASM: token arity");
                }
                for b in bs {
                    u64::try_from(number(b)?).map_err(|_| Error("QASM: token limit".into()))?;
                }
            }
            _ => bail!("QASM: unknown argument kind"),
        }
    }
    Ok(())
}
fn validate_histogram(counts: &V, total: u128) -> Result<()> {
    let histogram = field(counts, "histogram")?
        .as_object()
        .ok_or("QASM: histogram")?;
    let mut sum = 0u128;
    let mut max = 0;
    for (arity, count) in histogram {
        let arity = arity
            .parse::<usize>()
            .map_err(|_| Error("QASM: control arity".into()))?;
        let count = number(count)?;
        sum = sum.checked_add(count).ok_or("QASM: histogram overflow")?;
        if count > 0 {
            max = max.max(arity);
        }
    }
    if sum != total || number(field(counts, "max_controls")?)? != max as u128 {
        bail!("QASM: histogram/count mismatch");
    }
    Ok(())
}
pub(super) fn field<'a>(value: &'a V, name: &str) -> Result<&'a V> {
    value
        .get(name)
        .ok_or_else(|| Error(format!("QASM: missing {name}")))
}
pub(super) fn number(value: &V) -> Result<u128> {
    let text = value.as_str().ok_or("QASM: expected decimal string")?;
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || text.len() > 1 && text.starts_with('0')
    {
        bail!("QASM: noncanonical decimal");
    }
    text.parse()
        .map_err(|_| Error("QASM: integer overflow".into()))
}
pub(super) fn keys(value: &V, names: &[&str]) -> Result<()> {
    let fields = value.as_object().ok_or("QASM: expected object")?;
    if fields.len() != names.len() || names.iter().any(|n| !fields.contains_key(*n)) {
        bail!("QASM: metadata fields");
    }
    Ok(())
}
