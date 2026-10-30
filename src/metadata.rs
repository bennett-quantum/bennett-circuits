//! Recursive metadata schemas from ISA 0.8 §§9.1, 11–13, 16.
use crate::{Error, Result, bail, json::Value, object::Object};
use std::collections::BTreeMap;
fn error(path: &str, reason: &str) -> Error {
    Error(format!("metadata: {path}: {reason} (§9.1)"))
}
fn map<'a>(v: &'a Value, path: &str) -> Result<&'a BTreeMap<String, Value>> {
    v.as_object().ok_or_else(|| error(path, "expected object"))
}
fn fields<'a>(
    v: &'a Value,
    path: &str,
    allowed: &[&str],
    required: &[&str],
) -> Result<&'a BTreeMap<String, Value>> {
    let m = map(v, path)?;
    for k in m.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(error(path, &format!("unknown field {k}")));
        }
    }
    for k in required {
        if !m.contains_key(*k) {
            return Err(error(path, &format!("missing field {k}")));
        }
    }
    Ok(m)
}
fn array<'a>(v: &'a Value, path: &str) -> Result<&'a [Value]> {
    v.as_array().ok_or_else(|| error(path, "expected array"))
}
fn string<'a>(v: &'a Value, path: &str) -> Result<&'a str> {
    v.as_str().ok_or_else(|| error(path, "expected string"))
}
fn natural(v: &Value, path: &str) -> Result<u128> {
    let s = integer_text(v, path)?;
    s.parse()
        .map_err(|_| error(path, "host limit: integer exceeds u128"))
}
fn integer_text<'a>(v: &'a Value, path: &str) -> Result<&'a str> {
    let s = match v {
        Value::Int(s) | Value::Str(s) => s,
        _ => return Err(error(path, "expected nonnegative integer")),
    };
    if s.is_empty() || s.len() > 1 && s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error(path, "noncanonical integer"));
    }
    Ok(s)
}
fn integer_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
pub(crate) fn max_word(width: u32) -> String {
    let mut digits = vec![0u8];
    for _ in 0..width {
        let mut carry = 1;
        for d in &mut digits {
            let v = *d * 2 + carry;
            *d = v % 10;
            carry = v / 10;
        }
        if carry != 0 {
            digits.push(carry);
        }
    }
    digits.iter().rev().map(|d| char::from(b'0' + d)).collect()
}
fn hash(v: &Value, path: &str) -> Result<()> {
    let s = string(v, path)?;
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(path, "expected SHA-256 lowercase hex"));
    }
    Ok(())
}
fn choice<'a>(v: &'a Value, path: &str, choices: &[&str]) -> Result<&'a str> {
    let s = string(v, path)?;
    if !choices.contains(&s) {
        return Err(error(path, "invalid enum value"));
    }
    Ok(s)
}
fn decimal(v: &Value, path: &str) -> Result<()> {
    let s = match v {
        Value::Str(s) | Value::Int(s) => s,
        _ => return Err(error(path, "expected decimal")),
    };
    if s.parse::<f64>().is_err() || s.starts_with('-') || !s.parse::<f64>().unwrap().is_finite() {
        return Err(error(path, "expected finite nonnegative decimal"));
    }
    Ok(())
}
fn classical_type(v: &Value, path: &str) -> Result<()> {
    let s = string(v, path)?;
    if ["ptr", "f64", "token"].contains(&s) {
        return Ok(());
    }
    let n = s.strip_prefix('i').and_then(|s| s.parse::<u32>().ok());
    if n.is_some_and(|n| (1..=128).contains(&n) && s == format!("i{n}")) {
        return Ok(());
    }
    if let Some(n) = s
        .strip_prefix("qreg[")
        .and_then(|s| s.strip_suffix(']'))
        .and_then(|s| s.parse::<u32>().ok())
        && s == format!("qreg[{n}]")
    {
        return Ok(());
    }
    Err(error(path, "invalid classical type"))
}
pub(crate) fn signature(v: &Value, path: &str) -> Result<()> {
    let m = map(v, path)?;
    let kind = choice(
        m.get("kind").ok_or_else(|| error(path, "missing kind"))?,
        path,
        &[
            "cc",
            "r",
            "oracle",
            "lproc",
            "action",
            "perm_action",
            "data",
        ],
    )?;
    let mut keys = vec!["abi", "kind", "tier"];
    keys.extend(match kind {
        "oracle" => vec!["args", "in", "tgt", "fault", "mem"],
        "action" | "perm_action" => vec!["args", "in", "mut", "mem", "bounds"],
        "lproc" => vec!["params", "result"],
        "cc" | "r" => vec!["params", "results"],
        _ => vec!["size"],
    });
    fields(v, path, &keys, &keys)?;
    if string(&m["abi"], path)? != "0.8" {
        return Err(error(path, "unsupported ABI"));
    }
    let tier = match kind {
        "cc" => "cc",
        "r" => "r",
        "data" => "-",
        _ => "q",
    };
    if string(&m["tier"], path)? != tier {
        return Err(error(path, "kind/tier mismatch"));
    }
    for (k, v) in m {
        match k.as_str() {
            "in" | "mut" => {
                for w in array(v, path)? {
                    if natural(w, path)? == 0 {
                        return Err(error(path, "zero wire width"));
                    }
                }
            }
            "tgt" | "result" => {
                if natural(v, path)? == 0 {
                    return Err(error(path, "zero width"));
                }
            }
            "fault" => {
                if natural(v, path)? < 4 {
                    return Err(error(path, "fault width < 4 (§5.4)"));
                }
            }
            "params" | "results" | "args" => {
                for t in array(v, path)? {
                    if kind == "lproc" {
                        if natural(t, path)? == 0 {
                            return Err(error(path, "zero parameter width"));
                        }
                    } else {
                        classical_type(t, path)?;
                    }
                }
            }
            "mem" => {
                for r in array(v, path)? {
                    let r = fields(r, path, &["A", "W", "role"], &["A", "W", "role"])?;
                    natural(&r["A"], path)?;
                    if natural(&r["W"], path)? == 0 {
                        return Err(error(path, "zero cell width"));
                    }
                    choice(
                        &r["role"],
                        path,
                        if kind == "oracle" {
                            &["in"]
                        } else {
                            &["in", "mut"]
                        },
                    )?;
                }
            }
            "bounds" => {
                for n in array(v, path)? {
                    natural(n, path)?;
                }
            }
            "size" => {
                natural(v, path)?;
            }
            _ => {}
        }
    }
    Ok(())
}
fn versioned(v: &Value, path: &str) -> Result<()> {
    let m = fields(v, path, &["name", "version"], &["name", "version"])?;
    string(&m["name"], path)?;
    string(&m["version"], path)?;
    Ok(())
}
fn context_key(key: &str, p: &str) -> Result<()> {
    let Some((variant, controls)) = key.split_once(':') else {
        return Err(error(p, "context key requires variant:inherited_controls"));
    };
    choice(
        &Value::Str(variant.into()),
        p,
        &["forward", "adjoint", "controlled"],
    )?;
    if controls.parse::<u128>().is_err()
        || controls != "0" && controls.starts_with('0')
        || !controls.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(error(
            p,
            "context control count must be a canonical natural",
        ));
    }
    Ok(())
}
fn point_path(v: &Value, p: &str) -> Result<()> {
    for point in array(v, p)? {
        let point = fields(
            point,
            p,
            &["symbol", "point", "iteration"],
            &["symbol", "point", "iteration"],
        )?;
        hash(&point["symbol"], p)?;
        let id = string(&point["point"], p)?;
        let (block, ordinal) = id
            .rsplit_once(':')
            .ok_or_else(|| error(p, "invalid program point"))?;
        if block.is_empty()
            || !block.bytes().enumerate().all(|(i, b)| {
                b.is_ascii_alphabetic() || b == b'_' || i > 0 && (b.is_ascii_digit() || b == b'.')
            })
            || ordinal.parse::<u64>().is_err()
            || !ordinal.bytes().all(|b| b.is_ascii_digit())
            || ordinal != "0" && ordinal.starts_with('0')
        {
            return Err(error(p, "invalid program point"));
        }
        if point["iteration"].as_str() != Some("*") {
            for n in array(&point["iteration"], p)? {
                natural(n, p)?;
            }
        }
    }
    Ok(())
}
fn schedule(v: &Value) -> Result<()> {
    let p = "schedule";
    let m = fields(v, p, &["bindings", "entries"], &["bindings", "entries"])?;
    let keys = [
        "closure_hash",
        "target_hash",
        "region",
        "variant",
        "inherited_controls",
        "budget",
        "params_hash",
        "epsilon",
        "expansion",
        "scheduler",
    ];
    let b = fields(&m["bindings"], p, &keys, &keys)?;
    for k in ["closure_hash", "target_hash", "params_hash"] {
        hash(&b[k], p)?;
    }
    string(&b["region"], p)?;
    choice(&b["variant"], p, &["forward", "adjoint", "controlled"])?;
    natural(&b["inherited_controls"], p)?;
    decimal(&b["epsilon"], p)?;
    let budget = array(&b["budget"], p)?;
    if budget.len() != 4 {
        return Err(error(p, "budget requires four components"));
    }
    for n in budget {
        natural(n, p)?;
    }
    let e = fields(
        &b["expansion"],
        p,
        &["name", "version", "hash"],
        &["name", "version", "hash"],
    )?;
    for k in ["name", "version"] {
        string(&e[k], p)?;
    }
    hash(&e["hash"], p)?;
    versioned(&b["scheduler"], p)?;
    for entry in array(&m["entries"], p)? {
        let e = fields(
            entry,
            p,
            &[
                "path",
                "step",
                "phase",
                "generated",
                "kind",
                "cleanup",
                "gate",
                "alloc",
                "start",
            ],
            &["path", "step", "phase", "generated", "kind"],
        )?;
        point_path(&e["path"], p)?;
        for k in ["step", "generated"] {
            if e[k].as_str() != Some("*") {
                natural(&e[k], p)?;
            }
        }
        choice(&e["phase"], p, &["fwd", "rev", "*"])?;
        let kind = choice(&e["kind"], p, &["cleanup", "gate", "alloc", "start"])?;
        if ["cleanup", "gate", "alloc", "start"]
            .iter()
            .any(|k| e.contains_key(*k) != (*k == kind))
        {
            return Err(error(p, "entry payload must match kind"));
        }
        let payload = e
            .get(kind)
            .ok_or_else(|| error(p, "missing entry payload"))?;
        match kind {
            "cleanup" => {
                let c = fields(payload, p, &["realisation"], &["realisation"])?;
                if ![Some("reverse"), Some("recompute")].contains(&c["realisation"].as_str()) {
                    bail!("metadata: measurement-based cleanup is not admitted (§6.4; §9.1)");
                }
                if c["realisation"].as_str() == Some("recompute") {
                    bail!(
                        "unimplemented: §6.4 recompute eligibility and cleanup schedule matching; requires unpublished expansion library b-isa-spec-8r4 (milestone 2)"
                    );
                }
            }
            "gate" => {
                let g = fields(
                    payload,
                    p,
                    &["realisation", "epsilon_share"],
                    &["realisation", "epsilon_share"],
                )?;
                string(&g["realisation"], p)?;
                decimal(&g["epsilon_share"], p)?;
            }
            "alloc" => {
                let a = fields(payload, p, &["class"], &["class"])?;
                if string(&a["class"], p)? == "any" {
                    return Err(error(p, "allocation class must be concrete (§11)"));
                }
            }
            _ => {
                let t = fields(payload, p, &["time"], &["time"])?;
                decimal(&t["time"], p)?;
            }
        }
    }
    Ok(())
}
pub(crate) fn validate(o: &Object) -> Result<()> {
    if let Some(cert) = &o.certificate {
        certificate(cert)?;
    }
    for s in &o.symbols {
        signature(&s.signature, &format!("symbol @{}", s.name))?;
    }
    for i in &o.imports {
        signature(&i.signature, &format!("import @{}", i.name))?;
        if let Some(h) = &i.pinned_closure_hash {
            hash(&Value::Str(h.clone()), "import pin")?;
        }
    }
    if let Some(v) = &o.devices {
        let m = fields(v, "devices", &["models", "services"], &[])?;
        for k in ["models", "services"] {
            if let Some(v) = m.get(k) {
                for (n, v) in map(v, k)? {
                    let m = fields(v, n, &["version"], &["version"])?;
                    string(&m["version"], n)?;
                }
            }
        }
    }
    if let Some(v) = &o.layout {
        let keys = [
            "ptr_width",
            "H",
            "A_tab",
            "S_cells",
            "null_size",
            "heap_base",
            "heap_limit_max",
            "globals",
            "interface_regions",
            "block_image",
        ];
        let m = fields(v, "layout", &keys, &keys[..9])?;
        for k in &keys[..7] {
            natural(&m[*k], "layout")?;
        }
        for (n, g) in map(&m["globals"], "layout.globals")? {
            let g = fields(g, n, &["base", "size", "image"], &["base", "size", "image"])?;
            natural(&g["base"], n)?;
            natural(&g["size"], n)?;
            string(&g["image"], n)?;
        }
        for (name, rs) in map(&m["interface_regions"], "layout.interface_regions")? {
            for (r, v) in map(rs, name)? {
                let v = fields(v, r, &["base"], &["base"])?;
                natural(&v["base"], r)?;
            }
        }
        if let Some(v) = m.get("block_image") {
            string(v, "layout.block_image")?;
        }
    }
    if let Some(v) = &o.schedule {
        schedule(v)?;
    }
    if let Some(v) = &o.coverage {
        let m = fields(v, "coverage", &["entries"], &["entries"])?;
        for e in array(&m["entries"], "coverage.entries")? {
            let e = fields(
                e,
                "coverage.entry",
                &[
                    "point",
                    "source",
                    "classification",
                    "reason",
                    "fault_code",
                    "shim_hash",
                    "count",
                ],
                &["point", "source", "classification", "reason", "count"],
            )?;
            for k in ["source", "reason"] {
                string(&e[k], "coverage")?;
            }
            choice(
                &e["classification"],
                "coverage",
                &["lowered", "trapped", "shimmed"],
            )?;
            natural(&e["count"], "coverage")?;
            if let Some(v) = e.get("fault_code") {
                natural(v, "coverage")?;
            }
            if let Some(v) = e.get("shim_hash") {
                hash(v, "coverage")?;
            }
        }
    }
    Ok(())
}

fn domain(v: &Value) -> Result<()> {
    let p = "certificate.domain";
    let d = fields(
        v,
        p,
        &["bundles", "borrowed", "regions"],
        &["bundles", "borrowed", "regions"],
    )?;
    let mut names = std::collections::BTreeSet::new();
    for k in ["bundles", "borrowed"] {
        for v in array(&d[k], p)? {
            let w = fields(
                v,
                p,
                &["name", "width", "lo", "hi"],
                &["name", "width", "lo", "hi"],
            )?;
            if !names.insert(string(&w["name"], p)?) {
                return Err(error(p, "duplicate domain name"));
            }
            let width = natural(&w["width"], p)?;
            if width == 0 {
                return Err(error(p, "zero domain width"));
            }
            let lo = integer_text(&w["lo"], p)?;
            let hi = integer_text(&w["hi"], p)?;
            let width: u32 = width
                .try_into()
                .map_err(|_| error(p, "host limit: domain width exceeds u32"))?;
            // Divide the supplied integer rather than constructing 2^width:
            // even a very wide, sparse domain needs space only for its bound.
            let mut digits = hi.bytes().map(|b| b - b'0').collect::<Vec<_>>();
            let mut bits = 0u64;
            while digits.iter().any(|d| *d != 0) {
                let mut carry = 0;
                for d in &mut digits {
                    let v = carry * 10 + *d;
                    *d = v / 2;
                    carry = v % 2;
                }
                bits += 1;
            }
            if integer_cmp(lo, hi).is_gt() || bits > u64::from(width) {
                return Err(error(p, "domain bounds out of range"));
            }
        }
    }
    for v in array(&d["regions"], p)? {
        let r = fields(v, p, &["name", "cells"], &["name", "cells"])?;
        if !names.insert(string(&r["name"], p)?) {
            return Err(error(p, "duplicate domain name"));
        }
        if r["cells"].as_str() != Some("all") {
            let mut indices = std::collections::BTreeSet::new();
            for c in array(&r["cells"], p)? {
                let c = fields(c, p, &["index", "lo", "hi"], &["index", "lo", "hi"])?;
                if !indices.insert(integer_text(&c["index"], p)?) {
                    return Err(error(p, "duplicate domain cell"));
                }
                if integer_cmp(integer_text(&c["lo"], p)?, integer_text(&c["hi"], p)?).is_gt() {
                    return Err(error(p, "domain cell bounds reversed"));
                }
            }
        }
    }
    Ok(())
}
fn claim(v: &Value) -> Result<()> {
    let p = "certificate.claim";
    let m = map(v, p)?;
    let k = choice(
        m.get("kind")
            .ok_or_else(|| error(p, "missing claim kind"))?,
        p,
        &[
            "exhaustive",
            "sampled",
            "structural",
            "numerical",
            "channel",
        ],
    )?;
    let mut keys = vec!["kind"];
    keys.extend(match k {
        "exhaustive" => vec!["domain", "expected"],
        "sampled" => vec!["domain", "sampler", "expected"],
        "structural" => vec!["rule", "subclaims", "witness"],
        "numerical" => vec!["subject", "metric", "precision_bits", "bound"],
        _ => vec!["subject", "metric", "precision_bits", "bound", "dual"],
    });
    fields(
        v,
        p,
        &keys,
        if k == "sampled" {
            &["kind", "domain", "sampler"]
        } else {
            &keys
        },
    )?;
    if let Some(v) = m.get("domain") {
        domain(v)?;
    }
    if let Some(v) = m.get("expected")
        && v.as_str() != Some("oracle")
    {
        let expected = fields(v, p, &["table", "program"], &[])?;
        if expected.len() != 1 {
            return Err(error(p, "expected must be table or program"));
        }
        if let Some(v) = expected.get("program") {
            hash(v, p)?;
        }
        if let Some(v) = expected.get("table") {
            array(v, p)?;
        }
    }
    if let Some(v) = m.get("sampler") {
        let s = fields(
            v,
            p,
            &["algorithm", "seed", "count"],
            &["algorithm", "seed", "count"],
        )?;
        choice(&s["algorithm"], p, &["xoshiro256**"])?;
        let seed = array(&s["seed"], p)?;
        if seed.len() != 4 {
            return Err(error(p, "sampler seed requires four words"));
        }
        let mut nonzero = false;
        for word in seed {
            let n = natural(word, p)?;
            if n > u64::MAX as u128 {
                return Err(error(p, "seed word exceeds u64"));
            }
            nonzero |= n != 0;
        }
        if !nonzero {
            return Err(error(p, "all-zero seed"));
        }
        natural(&s["count"], p)?;
    }
    if let Some(rule) = m.get("rule") {
        let rule = choice(
            rule,
            p,
            &["translation_lemma", "inverse_pair_library", "adjoint_table"],
        )?;
        for i in array(&m["subclaims"], p)? {
            natural(i, p)?;
        }
        let w = &m["witness"];
        match rule {
            "translation_lemma" => {
                fields(
                    w,
                    p,
                    &["V", "W", "X", "B", "frame"],
                    &["V", "W", "X", "B", "frame"],
                )?;
                for k in ["V", "W", "X", "B", "frame"] {
                    array(get_field(w, k, p)?, p)?;
                }
            }
            "inverse_pair_library" => {
                let w = fields(
                    w,
                    p,
                    &["library_closure_hash", "claim_index"],
                    &["library_closure_hash", "claim_index"],
                )?;
                hash(&w["library_closure_hash"], p)?;
                natural(&w["claim_index"], p)?;
            }
            _ => {
                fields(w, p, &[], &[])?;
            }
        }
    }
    if let Some(v) = m.get("subject") {
        let s = fields(
            v,
            p,
            &[
                "closure_hash",
                "variant",
                "inherited_controls",
                "schedule_hash",
            ],
            &[
                "closure_hash",
                "variant",
                "inherited_controls",
                "schedule_hash",
            ],
        )?;
        hash(&s["closure_hash"], p)?;
        hash(&s["schedule_hash"], p)?;
        string(&s["variant"], p)?;
        natural(&s["inherited_controls"], p)?;
        choice(
            &m["metric"],
            p,
            if k == "numerical" {
                &["operator_norm"]
            } else {
                &["diamond"]
            },
        )?;
        natural(&m["precision_bits"], p)?;
        decimal(&m["bound"], p)?;
    }
    if let Some(v) = m.get("dual") {
        for v in array(v, p)? {
            let d = fields(v, p, &["Y", "mu"], &["Y", "mu"])?;
            decimal(&d["mu"], p)?;
            for entry in array(&d["Y"], p)? {
                let row = array(entry, p)?;
                if row.len() != 4 {
                    return Err(error(p, "interval matrix entry requires four endpoints"));
                }
                for e in row {
                    string(e, p)?;
                }
            }
        }
    }
    Ok(())
}
fn get_field<'a>(v: &'a Value, k: &str, p: &str) -> Result<&'a Value> {
    v.get(k).ok_or_else(|| error(p, "missing field"))
}
fn certificate(v: &Value) -> Result<()> {
    let p = "certificate";
    let m = fields(
        v,
        p,
        &["versions", "bindings", "producer", "regions", "composition"],
        &["versions", "bindings", "producer", "regions", "composition"],
    )?;
    let versions = fields(
        &m["versions"],
        p,
        &[
            "isa", "object", "launch", "sched", "cert", "coverage", "target",
        ],
        &["cert"],
    )?;
    for version in versions.values() {
        if string(version, p)? != "0.8" {
            return Err(error(p, "unsupported version"));
        }
    }
    let b = fields(
        &m["bindings"],
        p,
        &[
            "object_hash",
            "interface_hash",
            "closure_hash",
            "reference_program_hash",
            "dialect_mapping_version",
            "target_hash",
            "schedule_hash",
        ],
        &["object_hash", "interface_hash", "closure_hash"],
    )?;
    for (k, v) in b {
        if k == "dialect_mapping_version" {
            string(v, p)?;
        } else {
            hash(v, p)?;
        }
    }
    let producer = fields(
        &m["producer"],
        p,
        &["name", "version", "materialisation"],
        &["name", "version", "materialisation"],
    )?;
    for k in ["name", "version"] {
        string(&producer[k], p)?;
    }
    let mat = fields(
        &producer["materialisation"],
        p,
        &["llvm_version", "pipeline", "normaliser_version"],
        &["llvm_version", "pipeline", "normaliser_version"],
    )?;
    for k in ["llvm_version", "normaliser_version"] {
        string(&mat[k], p)?;
    }
    if mat["pipeline"].as_str().is_none() {
        for pass in array(&mat["pipeline"], p)? {
            string(pass, p)?;
        }
    }
    for (h, v) in map(&m["regions"], p)? {
        hash(&Value::Str(h.clone()), p)?;
        let r = fields(
            v,
            p,
            &[
                "kind",
                "signature",
                "bounds",
                "frame",
                "resources",
                "gates_used",
                "claims",
                "verification",
                "cleanup",
                "error",
                "definedness_assumptions",
            ],
            &["claims", "verification"],
        )?;
        if let Some(s) = r.get("signature") {
            signature(s, p)?;
        }
        if let Some(ns) = r.get("bounds") {
            for n in array(ns, p)? {
                natural(n, p)?;
            }
        }
        if let Some(rs) = r.get("resources") {
            for v in map(rs, p)?.values() {
                decimal(v, p)?;
            }
        }
        if let Some(gs) = r.get("gates_used") {
            for v in map(gs, p)?.values() {
                let g = fields(v, p, &["count", "scheme", "epsilon"], &["count"])?;
                natural(&g["count"], p)?;
                if let Some(e) = g.get("epsilon") {
                    decimal(e, p)?;
                }
                if let Some(s) = g.get("scheme") {
                    string(s, p)?;
                }
            }
        }
        let claims = array(&r["claims"], p)?;
        for c in claims {
            claim(c)?;
        }
        // Structural rules consume prior proof results. A self-supporting
        // cycle is not a proof, and an index cannot escape its claim array.
        fn visit_claim(n: usize, claims: &[Value], state: &mut [u8]) -> Result<()> {
            if n >= claims.len() {
                return Err(error("certificate.claim", "subclaim index out of range"));
            }
            match state[n] {
                1 => return Err(error("certificate.claim", "cyclic subclaims")),
                2 => return Ok(()),
                _ => {}
            }
            state[n] = 1;
            if let Some(subclaims) = claims[n].get("subclaims") {
                for i in array(subclaims, "certificate.claim")? {
                    let i = natural(i, "certificate.claim")?;
                    let i = usize::try_from(i)
                        .map_err(|_| error("certificate.claim", "subclaim index out of range"))?;
                    visit_claim(i, claims, state)?;
                }
            }
            state[n] = 2;
            Ok(())
        }
        let mut state = vec![0; claims.len()];
        for n in 0..claims.len() {
            visit_claim(n, claims, &mut state)?;
        }
        let verification = fields(
            &r["verification"],
            p,
            &[
                "oracle",
                "borrow_frames",
                "inverse_pairs",
                "unitary",
                "channel",
                "fault_rate",
            ],
            &[],
        )?;
        let index = |v: &Value| -> Result<()> {
            let n = natural(v, p)?;
            if n >= claims.len() as u128 {
                return Err(error(p, "claim index out of range"));
            }
            Ok(())
        };
        for (k, v) in verification {
            match k.as_str() {
                "oracle" => index(v)?,
                "borrow_frames" | "unitary" | "channel" => {
                    for i in array(v, p)? {
                        index(i)?;
                    }
                }
                "inverse_pairs" => {
                    for v in array(v, p)? {
                        let i = fields(v, p, &["g", "ginv", "claim"], &["g", "ginv", "claim"])?;
                        hash(&i["g"], p)?;
                        hash(&i["ginv"], p)?;
                        index(&i["claim"])?;
                    }
                }
                _ => {
                    let f = fields(
                        v,
                        p,
                        &[
                            "domain",
                            "faults_by_code",
                            "undefined_excluded",
                            "layout_dependent_excluded",
                        ],
                        &[
                            "domain",
                            "faults_by_code",
                            "undefined_excluded",
                            "layout_dependent_excluded",
                        ],
                    )?;
                    domain(&f["domain"])?;
                    map(&f["faults_by_code"], p)?;
                }
            }
        }
        if let Some(v) = r.get("cleanup") {
            for (key, v) in map(v, p)? {
                context_key(key, p)?;
                for e in array(v, p)? {
                    let e = fields(e, p, &["path", "realisation"], &["path", "realisation"])?;
                    if ![Some("reverse"), Some("recompute")].contains(&e["realisation"].as_str()) {
                        bail!("certificate: cleanup realisation invalid (§6.4; §9.1)");
                    }
                    if e["realisation"].as_str() == Some("recompute") {
                        bail!(
                            "unimplemented: §6.4 recompute eligibility and certificate cleanup matching; requires unpublished expansion library b-isa-spec-8r4 (milestone 2)"
                        );
                    }
                    point_path(&e["path"], p)?;
                }
            }
        }
        if let Some(v) = r.get("error") {
            for (key, v) in map(v, p)? {
                context_key(key, p)?;
                let e = fields(v, p, &["local", "inclusive"], &["local", "inclusive"])?;
                for v in e.values() {
                    for n in fields(v, p, &["operator_norm", "diamond"], &[])?.values() {
                        decimal(n, p)?;
                    }
                }
            }
        }
        if let Some(v) = r.get("definedness_assumptions") {
            for id in array(v, p)? {
                string(id, p)?;
            }
        }
        if r.contains_key("frame") {
            bail!("unimplemented: §9.4 certificate frame descriptor schema (milestone 2)");
        }
    }
    let comp = fields(
        &m["composition"],
        p,
        &["multiplicities", "epsilon_total", "method"],
        &["multiplicities", "epsilon_total", "method"],
    )?;
    choice(&comp["method"], p, &["sum_over_multiplicities"])?;
    for (key, v) in map(&comp["multiplicities"], p)? {
        let (region, context) = key.split_once(':').ok_or_else(|| {
            error(
                p,
                "multiplicity key requires hash:variant:inherited_controls",
            )
        })?;
        hash(&Value::Str(region.into()), p)?;
        context_key(context, p)?;
        natural(v, p)?;
    }
    for v in fields(&comp["epsilon_total"], p, &["diamond"], &["diamond"])?.values() {
        decimal(v, p)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn certificate_with_region(region: &str, multiplicities: &str) -> Value {
        let h = "0".repeat(64);
        Value::parse(&format!(r#"{{"versions":{{"cert":"0.8"}},"bindings":{{"object_hash":"{h}","interface_hash":"{h}","closure_hash":"{h}"}},"producer":{{"name":"test","version":"1","materialisation":{{"llvm_version":"none","pipeline":[],"normaliser_version":"1"}}}},"regions":{{"{h}":{region}}},"composition":{{"method":"sum_over_multiplicities","multiplicities":{multiplicities},"epsilon_total":{{"diamond":0}}}}}}"#)).unwrap()
    }
    #[test]
    fn certificate_context_keys_and_cleanup_paths_are_recursive_schemas() {
        certificate(&certificate_with_region(
            r#"{"claims":[],"verification":{},"cleanup":{"forward:0":[]}}"#,
            "{}",
        ))
        .unwrap();
        for region in [
            r#"{"claims":[],"verification":{},"cleanup":{"forward":[]}}"#,
            r#"{"claims":[],"verification":{},"cleanup":{"forward:-1":[]}}"#,
            r#"{"claims":[],"verification":{},"cleanup":{"forward:01":[]}}"#,
            r#"{"claims":[],"verification":{},"cleanup":{"sideways:1":[]}}"#,
            r#"{"claims":[],"verification":{},"cleanup":{"forward:0":[{"path":[0],"realisation":"reverse"}]}}"#,
            r#"{"claims":[],"verification":{},"error":{"forward":{"local":{},"inclusive":{}}}}"#,
        ] {
            assert!(
                certificate(&certificate_with_region(region, "{}")).is_err(),
                "{region}"
            );
        }
        for key in [
            "garbage",
            &format!("{}:forward", "0".repeat(64)),
            &format!("{}:forward:-1", "0".repeat(64)),
        ] {
            let multiplicities = format!(r#"{{"{key}":1}}"#);
            assert!(
                certificate(&certificate_with_region(
                    r#"{"claims":[],"verification":{}}"#,
                    &multiplicities
                ))
                .is_err(),
                "{key}"
            );
        }
    }
    #[test]
    fn domain_integers_are_not_limited_to_host_words() {
        let hi = "231584178474632390847141970017375815706539969331281128078915168015826259279871";
        let d = Value::parse(&format!(r#"{{"bundles":[{{"name":"x","width":257,"lo":"0","hi":"{hi}"}}],"borrowed":[],"regions":[]}}"#)).unwrap();
        domain(&d).unwrap();
        let bad = Value::parse(&d.to_jcs().replace("\"width\":257", "\"width\":256")).unwrap();
        assert!(domain(&bad).is_err());
    }
    #[test]
    fn domain_names_and_cell_indices_are_unique_and_bounds_are_ordered() {
        for text in [
            r#"{"bundles":[{"name":"x","width":1,"lo":0,"hi":1},{"name":"x","width":1,"lo":0,"hi":1}],"borrowed":[],"regions":[]}"#,
            r#"{"bundles":[],"borrowed":[],"regions":[{"name":"r","cells":[{"index":0,"lo":2,"hi":1}]}]}"#,
            r#"{"bundles":[],"borrowed":[],"regions":[{"name":"r","cells":[{"index":0,"lo":0,"hi":1},{"index":0,"lo":0,"hi":1}]}]}"#,
        ] {
            assert!(domain(&Value::parse(text).unwrap()).is_err(), "{text}");
        }
    }

    #[test]
    fn certificate_claim_dependencies_are_acyclic_and_in_range() {
        let h = "0".repeat(64);
        for subclaims in ["[1]", "[0]"] {
            let c=Value::parse(&format!(r#"{{"versions":{{"cert":"0.8"}},"bindings":{{"object_hash":"{h}","interface_hash":"{h}","closure_hash":"{h}"}},"producer":{{"name":"test","version":"1","materialisation":{{"llvm_version":"none","pipeline":[],"normaliser_version":"1"}}}},"regions":{{"{h}":{{"claims":[{{"kind":"structural","rule":"adjoint_table","subclaims":{subclaims},"witness":{{}}}}],"verification":{{}}}}}},"composition":{{"method":"sum_over_multiplicities","multiplicities":{{}},"epsilon_total":{{"diamond":0}}}}}}"#)).unwrap();
            assert!(certificate(&c).is_err(), "{subclaims}");
        }
    }
}
