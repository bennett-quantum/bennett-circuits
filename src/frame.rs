//! Typed frame records from ISA 0.9-draft §9.4 (ratified 4fa).
//! Acceptance requires an explicit draft opt-in; these are not ISA 0.8 fields.
use crate::{Error, Result, bail, json::Value};
use std::collections::BTreeMap;
fn fields<'a>(
    v: &'a Value,
    allowed: &[&str],
    required: &[&str],
) -> Result<&'a BTreeMap<String, Value>> {
    let m = v.as_object().ok_or("frame: expected record (§9.4)")?;
    if m.keys().any(|k| !allowed.contains(&k.as_str()))
        || required.iter().any(|k| !m.contains_key(*k))
    {
        bail!("frame: unknown/missing field (§9.1; §9.4)")
    }
    Ok(m)
}
fn expr(v: &Value) -> Result<()> {
    if let Value::Int(n) = v {
        if n.parse::<u64>().is_ok() {
            return Ok(());
        }
        bail!("frame: nonnegative count expression (§9.4)")
    }
    let m = v.as_object().ok_or("frame: expression (§8.1)")?;
    if m.contains_key("var") {
        fields(v, &["var"], &["var"])?;
        if m["var"].as_str() != Some("c") {
            bail!("frame: only inherited control variable c is allowed (§9.4)")
        }
        return Ok(());
    }
    fields(v, &["op", "args"], &["op", "args"])?;
    let args = m["args"]
        .as_array()
        .ok_or("frame: expression args (§8.1)")?;
    let arity = match m["op"].as_str() {
        Some("+" | "-" | "*" | "/" | "min" | "max") => 2,
        Some("ceil") => 1,
        _ => bail!("frame: expression operator (§8.1)"),
    };
    if args.len() != arity {
        bail!("frame: expression arity (§8.1)")
    }
    for a in args {
        expr(a)?
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub id: String,
    pub class: String,
    pub width: Value,
    pub role: Option<String>,
    pub state: Option<String>,
    pub scheme: Option<String>,
}
/// ISA 0.9-draft §9.4 B5 typed-record schema, including identity/alias checks.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub slots: Vec<Record>,
    pub scratch: Vec<Record>,
    pub history: Vec<Record>,
    pub borrow: Vec<Record>,
    pub ancillae: Vec<Record>,
    document: Value,
}
impl Frame {
    pub fn parse(v: &Value, draft: bool) -> Result<Self> {
        if !draft {
            bail!(
                "unimplemented: frame requires explicit --isa 0.9-draft; bennett-circuits-4fa (§9.4)"
            )
        }
        let keys = ["slots", "scratch", "history", "borrow", "ancillae"];
        let m = fields(
            v,
            &[
                "slots",
                "scratch",
                "history",
                "borrow",
                "ancillae",
                "live_ranges",
            ],
            &keys,
        )?;
        let mut ids = std::collections::BTreeSet::new();
        let mut parse_records = |k: &str| -> Result<Vec<Record>> {
            m[k].as_array()
                .ok_or("frame: required record array (§9.4)")?
                .iter()
                .map(|v| {
                    let mut keys = vec!["id", "class", "width"];
                    if k == "slots" {
                        keys.push("role");
                    }
                    if k == "scratch" || k == "ancillae" {
                        keys.push("state");
                    }
                    if k == "ancillae" {
                        keys.push("scheme");
                    }
                    let r = fields(v, &keys, &keys)?;
                    let text = |key: &str| -> Result<String> {
                        r[key]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .ok_or_else(|| Error(format!("frame: nonempty {key}")))
                    };
                    let id = text("id")?;
                    if !ids.insert(id.clone()) {
                        bail!("frame: duplicate identity (§9.4)")
                    }
                    expr(&r["width"])?;
                    let role = if k == "slots" {
                        let role = text("role")?;
                        if !["in", "tgt", "fault", "mut", "mem_in", "mem_mut"]
                            .contains(&role.as_str())
                        {
                            bail!("frame: slot role (§9.4)")
                        }
                        Some(role)
                    } else {
                        None
                    };
                    let state = if k == "scratch" || k == "ancillae" {
                        let state = text("state")?;
                        if !["clean", "dirty"].contains(&state.as_str()) {
                            bail!("frame: cleanliness (§9.4)")
                        }
                        Some(state)
                    } else {
                        None
                    };
                    Ok(Record {
                        id,
                        class: text("class")?,
                        width: r["width"].clone(),
                        role,
                        state,
                        scheme: if k == "ancillae" {
                            Some(text("scheme")?)
                        } else {
                            None
                        },
                    })
                })
                .collect()
        };
        let slots = parse_records("slots")?;
        let scratch = parse_records("scratch")?;
        let history = parse_records("history")?;
        let borrow = parse_records("borrow")?;
        let ancillae = parse_records("ancillae")?;
        if let Some(rs) = m.get("live_ranges") {
            let mut seen = BTreeMap::<String, Vec<(u64, u64)>>::new();
            for range in rs.as_array().ok_or("frame: live ranges array")? {
                let r = fields(
                    range,
                    &["frame", "start", "end"],
                    &["frame", "start", "end"],
                )?;
                let id = r["frame"].as_str().ok_or("frame: range identity")?;
                if !ids.contains(id) || slots.iter().any(|s| s.id == id) {
                    bail!("frame: range must name non-slot frame (§9.4)")
                }
                let start = r["start"].as_u64().ok_or("frame: range start")?;
                let end = r["end"].as_u64().ok_or("frame: range end")?;
                if start >= end
                    || seen
                        .entry(id.into())
                        .or_default()
                        .iter()
                        .any(|(a, b)| start < *b && *a < end)
                {
                    bail!("frame: overlapping/empty shared live ranges (§9.4)")
                }
                seen.get_mut(id).unwrap().push((start, end));
            }
        }
        Ok(Self {
            slots,
            scratch,
            history,
            borrow,
            ancillae,
            document: v.clone(),
        })
    }
    pub fn to_json(&self) -> Value {
        self.document.clone()
    }
}

/// Describe an already placed action's physical frame, using the published
/// 0.9-draft B5 records. No generated ordinals are invented for shared storage.
pub fn placed(
    p: &crate::placement::Placed,
    o: &crate::object::Object,
    draft: bool,
) -> Result<Frame> {
    if !draft {
        bail!("frame: explicit --isa 0.9-draft required (§9.4)")
    }
    let name = p
        .request
        .get("region")
        .and_then(Value::as_str)
        .ok_or("frame: placed region binding")?;
    let f = o.function(name).ok_or("frame: missing source function")?;
    if f.kind == crate::object::FnKind::Oracle
        || p.source.circuits.iter().any(|c| {
            o.functions.iter().any(|f| {
                f.kind == crate::object::FnKind::Oracle
                    && c.name.split('$').next() == Some(f.name.as_str())
            })
        })
    {
        bail!(
            "unimplemented: literal-oracle retained history classification; bennett-circuits-4fa (M4; §9.4)"
        )
    }
    let mut offset = 0;
    let mut slots = vec![];
    for (g, role) in [(0, "mut"), (1, "in"), (2, "mem")] {
        for (i, param) in f.groups[g].iter().enumerate() {
            let width = match param.ty {
                crate::object::Type::Wire(w) => w as usize,
                crate::object::Type::Region(a, w) => {
                    (1usize.checked_shl(a).ok_or("host limit: frame region")?)
                        .checked_mul(w as usize)
                        .ok_or("host limit: frame region")?
                }
                _ => bail!("frame: interface type (§9.4)"),
            };
            let class = p.classes.get(offset).ok_or("frame: missing class")?;
            if p.classes[offset..offset + width].iter().any(|k| k != class) {
                bail!("unimplemented: heterogeneous slot class; bennett-circuits-4fa (§9.4)")
            }
            slots.push(crate::certificate::obj([
                ("id", Value::Str(format!("{role}:{i}"))),
                (
                    "role",
                    Value::Str(if role == "mem" {
                        format!("mem_{}", param.role.as_deref().unwrap_or("in"))
                    } else {
                        role.into()
                    }),
                ),
                ("class", Value::Str(class.clone())),
                ("width", Value::Int(width.to_string())),
            ]));
            offset += width;
        }
    }
    let mut scratch = vec![];
    let mut borrow = vec![];
    let mut ancillae = vec![];
    for q in offset..p.classes.len() {
        let dirty = p.wires.iter().any(|w| w.wire == q && w.dirty);
        let borrowed = p
            .placement
            .lifetimes
            .iter()
            .any(|l| l.physical == q && l.kind == crate::ir::WsKind::Borrowed);
        let mut record = BTreeMap::from([
            ("id".into(), Value::Str(format!("physical:{q}"))),
            ("class".into(), Value::Str(p.classes[q].clone())),
            ("width".into(), Value::Int("1".into())),
        ]);
        if borrowed {
            borrow.push(Value::Object(record));
        } else {
            record.insert(
                "state".into(),
                Value::Str(if dirty { "dirty" } else { "clean" }.into()),
            );
            if q >= p.placement.flat.num_qubits {
                let schemes = p
                    .events
                    .iter()
                    .filter_map(|e| e.leaf.as_ref())
                    .filter(|l| {
                        l.gate.targets.contains(&q) || l.gate.controls.iter().any(|(n, _)| *n == q)
                    })
                    .filter_map(|l| l.scheme.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                if schemes.len() != 1 {
                    bail!(
                        "E_SCHEDULE_INFEASIBLE: shared scheme frame identity requires expansion library b-isa-spec-8r4 (§9.4)"
                    )
                }
                record.insert(
                    "scheme".into(),
                    Value::Str(schemes.into_iter().next().unwrap()),
                );
                ancillae.push(Value::Object(record));
            } else {
                scratch.push(Value::Object(record));
            }
        }
    }
    Frame::parse(
        &crate::certificate::obj([
            ("slots", Value::Array(slots)),
            ("scratch", Value::Array(scratch)),
            ("history", Value::Array(vec![])),
            ("borrow", Value::Array(borrow)),
            ("ancillae", Value::Array(ancillae)),
        ]),
        true,
    )
}

impl Frame {
    /// Check the descriptor's positional interface against the bound signature.
    pub(crate) fn validate_slots(&self, f: &crate::object::Function) -> Result<()> {
        use crate::object::{FnKind, Type};
        let groups = if f.kind == FnKind::Oracle {
            vec![(0, "in"), (1, "tgt"), (2, "fault"), (3, "mem")]
        } else {
            vec![(0, "mut"), (1, "in"), (2, "mem")]
        };
        let mut expected = vec![];
        for (g, role) in groups {
            for (i, p) in f.groups[g].iter().enumerate() {
                let (width, r) = match p.ty {
                    Type::Wire(w) => (u64::from(w), role.to_string()),
                    Type::Region(a, w) => (
                        1u64.checked_shl(a)
                            .and_then(|n| n.checked_mul(u64::from(w)))
                            .ok_or("host limit: frame region width")?,
                        format!("mem_{}", p.role.as_deref().unwrap_or("in")),
                    ),
                    _ => bail!("frame: signature slot type (§9.4)"),
                };
                expected.push((format!("{role}:{i}"), r, width));
            }
        }
        if self.slots.len() != expected.len() {
            bail!("frame: slot count differs from signature (§9.4)")
        }
        for (slot, (id, role, width)) in self.slots.iter().zip(expected) {
            let value = crate::target::evaluate(&slot.width, &BTreeMap::new(), &BTreeMap::new())
                .map_err(|e| {
                    Error(format!(
                        "frame: interface width must be signature-fixed; bennett-circuits-4fa: {e}"
                    ))
                })?
                .index()? as u64;
            if slot.id != id || slot.role.as_deref() != Some(&role) || value != width {
                bail!("frame: slot identity/order/width differs from signature (§9.4)")
            }
        }
        Ok(())
    }
}
