//! Verified incoming exhaustive T3 borrow claims (§§5.6,12).
//! All basis configurations of each frame are streamed, including arbitrary
//! entries on its non-borrowed wires. No sampled claim is promoted to proof.
use crate::{
    Error, Result, bail,
    flatten::{Flat, FlatGate, flatten_allocations},
    ir::{BaseGate, Circuit, Program, WsKind},
    json::Value,
    object::{Function, Object},
};
use std::collections::BTreeSet;
fn error(reason: &str) -> Error {
    Error(format!("borrow certificate: {reason} (§5.6; §12)"))
}
fn get<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
    v.get(k).ok_or_else(|| error(&format!("missing {k}")))
}
fn array(v: &Value) -> Result<&[Value]> {
    v.as_array().ok_or_else(|| error("expected array"))
}
fn full_word(v: &Value, name: &str, width: u32) -> bool {
    let number = |k: &str| -> Option<&str> {
        match v.get(k)? {
            Value::Int(s) | Value::Str(s) => Some(s),
            _ => None,
        }
    };
    v.get("name").and_then(Value::as_str) == Some(name)
        && number("width") == Some(width.to_string().as_str())
        && number("lo") == Some("0")
        && number("hi") == Some(crate::metadata::max_word(width).as_str())
}
fn domain(v: &Value, c: &Circuit, borrowed_widths: &[u32]) -> Result<()> {
    let bundles = array(get(v, "bundles")?)?;
    let words = c
        .mut_bundles
        .iter()
        .chain(&c.in_bundles)
        .collect::<Vec<_>>();
    if bundles.len() != words.len()
        || words.iter().any(|w| {
            bundles
                .iter()
                .filter(|v| full_word(v, &w.name, w.width))
                .count()
                != 1
        })
    {
        return Err(error("bundle domain is not the full interface D"));
    }
    let borrowed = array(get(v, "borrowed")?)?;
    // §12.2 K70: borrowed names are labels; positions bind lifetimes.
    // Include nested and adjoint-call lifetimes in the same allocation order
    // used by certificate::Space, even when this frame borrows locally too.
    if borrowed.len() != borrowed_widths.len()
        || borrowed.iter().zip(borrowed_widths).any(|(v, width)| {
            let label = v.get("name").and_then(Value::as_str).unwrap_or("");
            label.is_empty() || !full_word(v, label, *width)
        })
    {
        return Err(error("borrowed domain is not the full D"));
    }
    let regions = array(get(v, "regions")?)?;
    if regions.len() != c.mem.len()
        || c.mem.iter().any(|m| {
            regions
                .iter()
                .filter(|r| {
                    r.get("name").and_then(Value::as_str) == Some(&m.name)
                        && r.get("cells").and_then(Value::as_str) == Some("all")
                })
                .count()
                != 1
        })
    {
        return Err(error("region domain is not the full D"));
    }
    Ok(())
}
fn execute(gates: &[FlatGate], bits: &mut [bool]) {
    for g in gates {
        if g.controls.iter().all(|(q, p)| bits[*q] == *p) {
            match g.gate {
                BaseGate::X => bits[g.targets[0]] ^= true,
                BaseGate::Swap => bits.swap(g.targets[0], g.targets[1]),
                _ => unreachable!(),
            }
        }
    }
}
fn factor(gates: &[FlatGate], borrowed: &BTreeSet<usize>, width: usize) -> Result<()> {
    let touched = gates
        .iter()
        .flat_map(|g| g.targets.iter().chain(g.controls.iter().map(|(q, _)| q)))
        .copied()
        .collect::<BTreeSet<_>>();
    let wires = touched.iter().copied().collect::<Vec<_>>();
    let mut column = vec![false; wires.len()];
    loop {
        let mut input = vec![false; width];
        for (q, v) in wires.iter().zip(&column) {
            input[*q] = *v;
        }
        let mut actual = input.clone();
        execute(gates, &mut actual);
        if borrowed.iter().any(|q| actual[*q] != input[*q]) {
            return Err(error("frame does not restore B"));
        }
        let mut zero_b = input;
        for q in borrowed {
            zero_b[*q] = false;
        }
        execute(gates, &mut zero_b);
        if touched
            .iter()
            .any(|q| !borrowed.contains(q) && actual[*q] != zero_b[*q])
        {
            return Err(error("frame output depends on B: P_F does not factor"));
        }
        let mut carry = true;
        for bit in &mut column {
            if carry {
                *bit = !*bit;
                carry = !*bit;
            }
        }
        if carry {
            break;
        }
    }
    Ok(())
}
pub(crate) fn verify(
    o: &Object,
    f: &Function,
    p: &Program,
    flat: &Flat,
    b: &BTreeSet<usize>,
) -> Result<()> {
    let c = p.top_circuit()?;
    let (_, _, allocations) = flatten_allocations(p)?;
    let borrowed_widths = allocations
        .iter()
        .filter(|a| a.kind == WsKind::Borrowed)
        .map(|a| u32::try_from(a.wires.len()).map_err(|_| error("borrow width host limit")))
        .collect::<Result<Vec<_>>>()?;
    let cert = o
        .certificate
        .as_ref()
        .ok_or_else(|| error("missing certificate"))?;
    let identities = o.identity()?;
    let binding = get(cert, "bindings")?;
    if get(binding, "object_hash")?.as_str() != Some(&o.object_hash()) {
        return Err(error("object_hash binding mismatch"));
    }
    let closure = identities
        .get(&f.name)
        .and_then(|h| h.closure_hash.as_deref())
        .ok_or_else(|| error("unresolved region closure"))?;
    // The certificate root can be an enclosing region; its identity must
    // resolve in this object even when this frame belongs to a callee.
    let root = get(binding, "closure_hash")?
        .as_str()
        .ok_or_else(|| error("closure hash encoding"))?;
    let root_identity = identities
        .values()
        .find(|h| h.closure_hash.as_deref() == Some(root))
        .ok_or_else(|| error("root closure binding mismatch"))?;
    if get(binding, "interface_hash")?.as_str() != Some(&root_identity.interface_hash) {
        return Err(error("interface hash binding mismatch"));
    }
    let region = get(get(cert, "regions")?, closure)?;
    let claims = array(get(region, "claims")?)?;
    let indices = array(get(get(region, "verification")?, "borrow_frames")?)?;
    let mut accepted = false;
    for index in indices {
        let index = index
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| error("claim index encoding"))?;
        let claim = claims
            .get(index)
            .ok_or_else(|| error("claim index out of range"))?;
        match get(claim, "kind")?.as_str() {
            Some("exhaustive") => {
                domain(get(claim, "domain")?, c, &borrowed_widths)?;
                let expected = get(claim, "expected")?;
                if expected.get("program").and_then(Value::as_str) != Some(closure)
                    && !(o.draft_encodings
                        && (expected.get("table").is_some()
                            || expected.get("program").and_then(Value::as_str).is_some()))
                {
                    bail!("unimplemented: §12 borrow expected-table/external-program replay");
                }
                // Object::check/hash replay every draft table/program relation
                // through certificate::verify after structural validation.
                // Factorisation is independently proved below; neither a
                // linked hash nor a claimed table establishes it by itself.
                accepted = true;
            }
            Some("sampled") => {
                return Err(error("sampled borrow evidence is not a full-D T3 proof"));
            }
            Some("structural") => {
                if claim.get("rule").and_then(Value::as_str) == Some("inverse_pair_library") {
                    bail!(
                        "unimplemented: §12 borrow inverse-pair library replay; reference expansion library (§11.1)"
                    )
                }
                bail!("unimplemented: §12 borrow library/structural claim replay")
            }
            _ => return Err(error("borrow verification must reference a T3 proof")),
        }
    }
    if !accepted {
        return Err(error("no verified exhaustive borrow claim"));
    }
    // Maximal permutation-only components. A non-permutation instruction
    // touching B is always invalid; no certificate can change that rule.
    let mut start = 0;
    for end in 0..=flat.gates.len() {
        let boundary = end == flat.gates.len()
            || !matches!(flat.gates[end].gate, BaseGate::X | BaseGate::Swap);
        if !boundary {
            continue;
        }
        if end < flat.gates.len()
            && flat.gates[end]
                .targets
                .iter()
                .chain(flat.gates[end].controls.iter().map(|(q, _)| q))
                .any(|q| b.contains(q))
        {
            return Err(error("non-permutation instruction touches B"));
        }
        let gates = &flat.gates[start..end];
        if gates.iter().any(|g| {
            g.targets
                .iter()
                .chain(g.controls.iter().map(|(q, _)| q))
                .any(|q| b.contains(q))
        }) {
            factor(gates, b, flat.num_qubits)?;
        }
        start = end + 1;
    }
    Ok(())
}
