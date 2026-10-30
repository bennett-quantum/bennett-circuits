//! §9.3 resolution and linked-object closure registry. No external source code.
use crate::{
    Error, Result, bail,
    json::Value,
    object::{Function, Object},
};
use std::collections::BTreeMap;
#[derive(Debug)]
pub struct Registry {
    pub object: Object,
    closures: BTreeMap<String, String>,
}
fn merge(a: &mut Option<Value>, b: &Option<Value>) -> Result<()> {
    fn entry(a: &mut Value, b: &Value) -> Result<()> {
        if a == b {
            return Ok(());
        }
        if let (Value::Object(a), Value::Object(b)) = (a, b) {
            for (k, v) in b {
                if let Some(old) = a.get_mut(k) {
                    entry(old, v)?
                } else {
                    a.insert(k.clone(), v.clone());
                }
            }
            Ok(())
        } else {
            bail!("link: conflicting devices/layout entries (§9.3)")
        }
    }
    if let Some(b) = b {
        if let Some(a) = a {
            entry(a, b)?
        } else {
            *a = Some(b.clone())
        }
    }
    Ok(())
}
fn rebind(v: &Value, m: &BTreeMap<String, String>) -> Value {
    fn walk(v: &Value, m: &BTreeMap<String, String>, field: &str) -> Value {
        match v {
            Value::Str(s)
                if [
                    "closure_hash",
                    "interface_hash",
                    "reference_program_hash",
                    "library_closure_hash",
                    "program",
                    "g",
                    "ginv",
                    "symbol",
                ]
                .contains(&field) =>
            {
                Value::Str(m.get(s).unwrap_or(s).clone())
            }
            Value::Array(a) => {
                if ["point", "V", "W", "frame"].contains(&field)
                    && a.len() == 3
                    && a[0].as_str().is_some()
                    && a[1].as_str().is_some()
                    && a[2].as_u64().is_some()
                {
                    let mut a = a.clone();
                    let h = a[0].as_str().unwrap();
                    a[0] = Value::Str(m.get(h).map_or(h, String::as_str).into());
                    Value::Array(a)
                } else {
                    Value::Array(a.iter().map(|v| walk(v, m, field)).collect())
                }
            }
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, v)| {
                        let key = if field == "regions" {
                            m.get(k).unwrap_or(k).clone()
                        } else if field == "multiplicities" {
                            if let Some((h, suffix)) = k.split_once(':') {
                                format!("{}:{suffix}", m.get(h).map_or(h, String::as_str))
                            } else {
                                k.clone()
                            }
                        } else {
                            k.clone()
                        };
                        (key, walk(v, m, k))
                    })
                    .collect(),
            ),
            _ => v.clone(),
        }
    }
    walk(v, m, "")
}
impl Registry {
    pub fn link(root: &Object, libraries: &[Object]) -> Result<Self> {
        let mut linked = root.clone();
        let mut imports = root.imports.clone();
        let mut duplicates = vec![];
        // Validate schemas and bodies of each input, but leave the pending
        // obligations pending until the complete object is assembled.
        for o in std::iter::once(root).chain(libraries) {
            if o.schedule.is_some() {
                bail!(
                    "E_SCHEDULE_INFEASIBLE: linked schedule composition requires expansion library b-isa-spec-8r4 (§9.3)"
                )
            }
            o.check()?;
        }
        for lib in libraries {
            if lib.header != root.header {
                bail!("link: incompatible headers (§9.3)")
            }
            merge(&mut linked.layout, &lib.layout)?;
            merge(&mut linked.devices, &lib.devices)?;
            for s in &lib.symbols {
                if let Some(old) = linked.symbol(&s.name) {
                    if old.signature != s.signature {
                        bail!("link: signature conflict under {} (§9.3)", s.name)
                    }
                    duplicates.push((
                        s.name.clone(),
                        lib.function(&s.name).cloned(),
                        lib.data_entry(&s.name).cloned(),
                    ));
                } else {
                    let mut s = s.clone();
                    s.index = linked.symbols.len().to_string();
                    linked.symbols.push(s);
                    if let Some(f) = lib.function(&linked.symbols.last().unwrap().name) {
                        linked.functions.push(f.clone());
                    }
                    if let Some(d) = lib.data_entry(&linked.symbols.last().unwrap().name) {
                        linked.data.push(d.clone());
                    }
                }
            }
            imports.extend(lib.imports.clone());
        }
        for import in &imports {
            let symbol = linked.symbol(&import.name).ok_or_else(|| {
                Error(format!(
                    "link: unresolved @{}; obligations pending (§9.3)",
                    import.name
                ))
            })?;
            if symbol.signature != import.signature {
                bail!("link: signature mismatch @{} (§9.3)", import.name)
            }
        }
        linked.imports.clear();
        // Original certificates retain original object bindings. Never treat
        // them as a proof of the newly linked object.
        linked.certificate = None;
        linked.schedule = None;
        linked = linked.materialize()?;
        // Rebind verified certificate records to the assembled object's
        // closures. Keep the actual claims and invocations; never copy verdicts.
        let identities = linked.identity()?;
        for (name, alternate_fn, alternate_data) in duplicates {
            let mut alternate = linked.clone();
            if let Some(f) = alternate_fn {
                let old = alternate
                    .functions
                    .iter_mut()
                    .find(|f| f.name == name)
                    .ok_or("link: duplicate kind mismatch")?;
                *old = f;
            }
            if let Some(d) = alternate_data {
                let old = alternate
                    .data
                    .iter_mut()
                    .find(|d| d.name == name)
                    .ok_or("link: duplicate kind mismatch")?;
                *old = d;
            }
            if identities[&name].closure_hash != alternate.identity()?[&name].closure_hash {
                bail!("link: unequal closure definitions under {name} (§9.3)")
            }
        }

        let mut composed = None::<Value>;
        let mut records = BTreeMap::new();
        let mut coverage = vec![];
        for source in std::iter::once(root).chain(libraries) {
            linked.draft_encodings |= source.draft_encodings;
            let mut mapping = BTreeMap::new();
            for (name, old) in source.identity()? {
                if let Some(new) = identities.get(&name) {
                    mapping.insert(old.interface_hash, new.interface_hash.clone());
                    if let (Some(a), Some(b)) = (old.closure_hash, &new.closure_hash) {
                        mapping.insert(a, b.clone());
                    }
                }
            }
            if let Some(v) = &source.coverage {
                let rebound = rebind(v, &mapping);
                for entry in rebound
                    .get("entries")
                    .and_then(Value::as_array)
                    .ok_or("link: coverage entries")?
                {
                    if !coverage.contains(entry) {
                        coverage.push(entry.clone());
                    }
                }
            }
            let Some(certificate) = &source.certificate else {
                continue;
            };
            let cert = rebind(certificate, &mapping);
            if let Some(previous) = &composed {
                for key in ["target_hash", "dialect_mapping_version"] {
                    if previous.get("bindings").and_then(|v| v.get(key))
                        != cert.get("bindings").and_then(|v| v.get(key))
                    {
                        bail!("link: incompatible certificate {key} (§12.3)")
                    }
                }
            }
            for (h, r) in cert
                .get("regions")
                .and_then(Value::as_object)
                .ok_or("link: certificate regions (§12.3)")?
            {
                if let Some(old) = records.insert(h.clone(), r.clone())
                    && old != *r
                {
                    bail!(
                        "unimplemented: composition of distinct certificates for equal closure; bennett-circuits-4pf (§12.3)"
                    )
                }
            }
            composed.get_or_insert(cert);
        }
        if !coverage.is_empty() {
            linked.coverage = Some(crate::certificate::obj([(
                "entries",
                Value::Array(coverage),
            )]));
        }
        if let Some(Value::Object(mut cert)) = composed {
            let root_name = &root
                .functions
                .first()
                .ok_or("link: certificate root function missing")?
                .name;
            let root_hash = identities
                .get(root_name)
                .ok_or("link: missing root identity")?;
            cert.insert("regions".into(), Value::Object(records));
            let Value::Object(bindings) = cert
                .get_mut("bindings")
                .ok_or("link: certificate bindings")?
            else {
                bail!("link: bindings record")
            };
            bindings.insert("object_hash".into(), Value::Str(linked.object_hash()));
            bindings.insert(
                "reference_program_hash".into(),
                Value::Str(root_hash.interface_hash.clone()),
            );
            bindings.insert(
                "interface_hash".into(),
                Value::Str(root_hash.interface_hash.clone()),
            );
            bindings.insert(
                "closure_hash".into(),
                Value::Str(
                    root_hash
                        .closure_hash
                        .clone()
                        .ok_or("link: unresolved root")?,
                ),
            );
            linked.certificate = Some(Value::Object(cert));
            linked = linked.materialize()?;
        }
        linked = linked.materialize()?;
        let report = linked.check()?;
        for import in &imports {
            if let Some(pin) = &import.pinned_closure_hash
                && report.symbols[&import.name].closure_hash.as_ref() != Some(pin)
            {
                bail!("link: pinned closure mismatch @{} (§9.3)", import.name)
            }
        }
        let closures = report
            .symbols
            .into_iter()
            .filter_map(|(name, h)| h.closure_hash.map(|h| (h, name)))
            .collect();
        Ok(Self {
            object: linked,
            closures,
        })
    }
    pub fn resolve(&self, hash: &str) -> Result<&Function> {
        self.closures
            .get(hash)
            .and_then(|n| self.object.function(n))
            .ok_or_else(|| {
                Error(format!(
                    "closure registry: unresolved {hash} (§12.3; bennett-circuits-4pf)"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rebinding_only_rewrites_identity_fields_and_source_points() {
        let value=Value::parse(r#"{"regions":{"old":{"claims":[{"expected":{"program":"old"},"invocation":{"args":["old"]}}],"frame":{"slots":[{"class":"old"}]}}},"point":["old","entry",0]}"#).unwrap();
        let actual = rebind(&value, &BTreeMap::from([("old".into(), "new".into())]));
        let expected=Value::parse(r#"{"regions":{"new":{"claims":[{"expected":{"program":"new"},"invocation":{"args":["old"]}}],"frame":{"slots":[{"class":"old"}]}}},"point":["new","entry",0]}"#).unwrap();
        assert_eq!(actual, expected);
    }
}
