//! §13 coverage in the object's own format. Source ids must be supplied by
//! the dialect mapping; the circuit producer never invents dialect stable ids.
use crate::{
    Result, bail,
    json::Value,
    object::{Instr, Object},
};
use std::collections::BTreeMap;
/// Enumerate every explicit trapped source construct, with its actual cause
/// (§4.9: `fault 0` records FAULT_UB_TRAP=4). The source-id map is keyed by the
/// JCS program-point id; absent mappings refuse before producing coverage.
pub fn traps(o: &Object, sources: &BTreeMap<String, String>) -> Result<Value> {
    let ids = o.identity()?;
    let mut entries = vec![];
    fn visit(
        is: &[Instr],
        hash: &str,
        block: &str,
        ordinal: &mut usize,
        sources: &BTreeMap<String, String>,
        entries: &mut Vec<Value>,
    ) -> Result<()> {
        for i in is {
            let point = Value::Array(vec![
                Value::Str(hash.into()),
                Value::Str(block.into()),
                Value::Int(ordinal.to_string()),
            ]);
            *ordinal += 1;
            if matches!(i.opcode.as_str(), "fault" | "lift.faultif") {
                let Some(crate::object::Operand::Int(code)) = i.operands.last() else {
                    bail!("coverage: explicit fault cause required (§4.9)")
                };
                let code = if code == "0" { "4" } else { code };
                let source = sources
                    .get(&point.to_jcs())
                    .filter(|s| !s.is_empty())
                    .ok_or("coverage: missing dialect-spec stable source id (§13)")?;
                entries.push(crate::certificate::obj([
                    ("point", point),
                    ("source", Value::Str(source.clone())),
                    ("classification", Value::Str("trapped".into())),
                    (
                        "reason",
                        Value::Str(format!("explicit {} (§4.9)", i.opcode)),
                    ),
                    ("fault_code", Value::Str(code.into())),
                    ("count", Value::Int("1".into())),
                ]));
            }
            for child in &i.children {
                visit(child, hash, block, ordinal, sources, entries)?;
            }
        }
        Ok(())
    }
    for f in &o.functions {
        for b in &f.blocks {
            visit(
                &b.instrs,
                &ids[&f.name].interface_hash,
                &b.name,
                &mut 0,
                sources,
                &mut entries,
            )?;
        }
    }
    Ok(crate::certificate::obj([(
        "entries",
        Value::Array(entries),
    )]))
}
