//! Exact basis proofs for small specialised permutation circuits. No sampling
//! and no floating arithmetic: X/SWAP basis evolution is Boolean. ISA §12
//! certificate serialization/bindings remain milestone 4.
use crate::{
    Error, Result, bail,
    flatten::{Flat, flatten},
    ir::BaseGate,
    object::{FnKind, Object},
    oracle::{Budget, specialise},
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InversePairProof {
    pub g: String,
    pub ginv: String,
    pub object_hash: String,
    pub g_closure_hash: String,
    pub ginv_closure_hash: String,
    pub budget: Budget,
    pub width: u32,
}
/// Exact computational-basis execution of an X/SWAP circuit. The caller
/// supplies every interface bit, including observer outputs for diagnostics.
pub fn basis(flat: &Flat, input: &[bool]) -> Result<Vec<bool>> {
    if input.len() != flat.interface.len() {
        bail!("basis executor: interface width mismatch")
    }
    if flat
        .gates
        .iter()
        .any(|g| !matches!(g.gate, BaseGate::X | BaseGate::Swap))
    {
        bail!("unimplemented: non-permutation CLI state-vector execution (milestone 6)")
    }
    let mut bits = vec![false; flat.num_qubits];
    for (q, b) in flat.interface.iter().zip(input) {
        bits[*q] = *b;
    }
    for g in &flat.gates {
        if g.controls.iter().all(|(q, p)| bits[*q] == *p) {
            match g.gate {
                BaseGate::X => bits[g.targets[0]] ^= true,
                BaseGate::Swap => bits.swap(g.targets[0], g.targets[1]),
                _ => unreachable!(),
            }
        }
    }
    if flat.workspace.iter().any(|q| bits[*q]) {
        bail!("basis executor: workspace is not clean (§5.1/§5.2)")
    }
    Ok(flat.interface.iter().map(|q| bits[*q]).collect())
}
/// Execute the compiled bounded oracle, including active-row observers, on
/// one basis input. Result fields are little-endian bit vectors.
pub fn oracle_basis(
    obj: &Object,
    name: &str,
    budget: Budget,
    args: &[crate::oracle::Argument],
    inputs: &[Vec<bool>],
    memory: &std::collections::BTreeMap<String, Vec<bool>>,
) -> Result<(Vec<bool>, Vec<bool>, u64)> {
    // Basis diagnostics have classical inputs. Once a prefix has halted,
    // additional steps are HALT padding (§3.6/§5.2) and cannot change the
    // committed tuple. This optimisation is only for this diagnostic
    // executor: exported circuits still implement their exact fixed budget.
    let mut prefix = budget.steps.min(1);
    loop {
        let result = oracle_basis_prefix(
            obj,
            name,
            Budget {
                steps: prefix,
                ..budget
            },
            args,
            inputs,
            memory,
        )?;
        let budget_fault = result.1.first() == Some(&true) && result.1.iter().skip(1).all(|b| !*b);
        if prefix == budget.steps || result.2 < prefix || !budget_fault {
            return Ok(result);
        }
        prefix = prefix.saturating_mul(2).min(budget.steps);
    }
}
fn oracle_basis_prefix(
    obj: &Object,
    name: &str,
    budget: Budget,
    args: &[crate::oracle::Argument],
    inputs: &[Vec<bool>],
    memory: &std::collections::BTreeMap<String, Vec<bool>>,
) -> Result<(Vec<bool>, Vec<bool>, u64)> {
    let f = obj
        .function(name)
        .ok_or_else(|| Error(format!("oracle executor: unknown @{name}")))?;
    if inputs.len() != f.groups[0].len() {
        bail!("oracle executor: input arity (§3.3)")
    }
    let n = f.groups[1][0].ty.wire_width().unwrap() as usize;
    let fw = f.groups[2][0].ty.wire_width().unwrap() as usize;
    let mut known = std::collections::BTreeMap::new();
    for (p, bits) in f.groups[0].iter().zip(inputs) {
        for (i, bit) in bits.iter().enumerate() {
            known.insert(crate::ir::QubitRef::new(&p.name, i as u32), *bit);
        }
    }
    for (name, bits) in memory {
        for (i, bit) in bits.iter().enumerate() {
            known.insert(crate::ir::QubitRef::new(name, i as u32), *bit);
        }
    }
    let (program, probes) = crate::oracle::specialise_probe(obj, name, budget, args, known)?;
    let flat = flatten(&program)?;
    let mut input = vec![false; n + fw + probes];
    for (bits, p) in inputs.iter().zip(&f.groups[0]) {
        if bits.len() != p.ty.wire_width().unwrap() as usize {
            bail!("oracle executor: input width (§3.3)")
        }
        input.extend(bits);
    }
    for m in &program.circuits[0].mem {
        if let Some(bits) = memory.get(&m.name) {
            if bits.len() != m.width() as usize {
                bail!("oracle executor: memory width (§3.3)")
            }
            input.extend(bits);
        } else {
            input.resize(input.len() + m.width() as usize, false);
        }
    }
    if memory
        .keys()
        .any(|n| !program.circuits[0].mem.iter().any(|m| m.name == *n))
    {
        bail!("oracle executor: unknown memory region")
    }
    let result = basis(&flat, &input)?;
    if result[n + fw + probes..] != input[n + fw + probes..] {
        bail!("oracle executor: input or memory not restored by prefix reversal (§5.2)");
    }
    let steps = result[n + fw..n + fw + probes]
        .iter()
        .filter(|b| **b)
        .count() as u64;
    Ok((result[..n].to_vec(), result[n..n + fw].to_vec(), steps))
}
fn run(flat: &Flat, x: &[bool], n: usize, fault: usize) -> Result<Vec<bool>> {
    let mut bits = vec![false; flat.num_qubits];
    for (i, b) in x.iter().enumerate() {
        bits[flat.interface[n + fault + i]] = *b;
    }
    for g in &flat.gates {
        if g.controls.iter().all(|(q, p)| bits[*q] == *p) {
            match g.gate {
                BaseGate::X => bits[g.targets[0]] ^= true,
                BaseGate::Swap => bits.swap(g.targets[0], g.targets[1]),
                _ => bail!("qinplace: oracle has a non-permutation gate (§4.5/§5.2)"),
            }
        }
    }
    if flat.workspace.iter().any(|q| bits[*q]) {
        bail!("qinplace: oracle workspace did not reverse (§4.5/§5.2)")
    }
    if flat.interface[n..n + fault].iter().any(|q| bits[*q]) {
        bail!("qinplace: oracle is not fault-free under the actual token (§4.5)")
    }
    if x.iter()
        .enumerate()
        .any(|(i, b)| bits[flat.interface[n + fault + i]] != *b)
    {
        bail!("qinplace: oracle modified an input (§4.5/§5.2)")
    }
    Ok(flat.interface[..n].iter().map(|q| bits[*q]).collect())
}
/// Exhaustively prove both compositions and absence of faults for the actual
/// token. Enumeration is streaming at arbitrary widths; it has no small-domain
/// cutoff and never turns a sample into a proof. Large domains need a verified
/// library certificate for practical compilation (checker support remains M2).
pub fn inverse_pair(obj: &Object, g: &str, ginv: &str, budget: Budget) -> Result<InversePairProof> {
    let shape = |name: &str| -> Result<(u32, u32)> {
        let f = obj
            .function(name)
            .ok_or_else(|| Error(format!("qinplace: missing oracle @{name} (§4.5)")))?;
        if f.kind != FnKind::Oracle
            || f.groups[0].len() != 1
            || !f.groups[3].is_empty()
            || !f.groups[4].is_empty()
        {
            bail!("qinplace: ISA 0.8 requires a unary n-bit oracle with no extra interface (§4.5)")
        }
        let n = f.groups[0][0]
            .ty
            .wire_width()
            .ok_or_else(|| Error("qinplace: input width (§4.5)".into()))?;
        if f.groups[1][0].ty.wire_width() != Some(n) {
            bail!("qinplace: input/target widths differ (§4.5)")
        }
        Ok((n, f.groups[2][0].ty.wire_width().unwrap()))
    };
    let (n, f) = shape(g)?;
    if shape(ginv)? != (n, f) {
        bail!("qinplace: inverse signature mismatch (§4.5)")
    }
    let forward = flatten(&specialise(obj, g, budget, &[])?)?;
    let inverse = flatten(&specialise(obj, ginv, budget, &[])?)?;
    if forward
        .gates
        .iter()
        .chain(&inverse.gates)
        .any(|g| !matches!(g.gate, BaseGate::X | BaseGate::Swap))
    {
        bail!("qinplace: non-permutation oracle realisation (§4.5)")
    }
    let mut x = vec![false; n as usize];
    loop {
        let gx = run(&forward, &x, n as usize, f as usize)?;
        let ix = run(&inverse, &x, n as usize, f as usize)?;
        if run(&inverse, &gx, n as usize, f as usize)? != x
            || run(&forward, &ix, n as usize, f as usize)? != x
        {
            bail!("qinplace: g∘ginv or ginv∘g differs from identity at {x:?} (§4.5)")
        }
        let mut carry = true;
        for bit in &mut x {
            if carry {
                *bit = !*bit;
                carry = !*bit;
            }
        }
        if carry {
            break;
        }
    }
    let identities = obj.identity()?;
    let closure = |name: &str| {
        identities
            .get(name)
            .and_then(|h| h.closure_hash.clone())
            .ok_or_else(|| {
                Error(format!(
                    "unimplemented: qinplace unresolved closure @{name}; decision bennett-circuits-4pf (milestone 2)"
                ))
            })
    };
    Ok(InversePairProof {
        g: g.into(),
        ginv: ginv.into(),
        object_hash: obj.object_hash(),
        g_closure_hash: closure(g)?,
        ginv_closure_hash: closure(ginv)?,
        budget,
        width: n,
    })
}
