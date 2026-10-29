//! Sparse exact simulator of parsed exported gate statements. No ISA dependency.
use super::exact::{Amp, D};
use bennett_circuits::{
    flatten::{Flat, FlatGate},
    ir::BaseGate as G,
};
use std::collections::BTreeMap;
fn put<K: Ord>(state: &mut BTreeMap<K, Amp>, key: K, value: Amp) {
    let value = state.get(&key).unwrap_or(&Amp::zero()).add(&value);
    if value.is_zero() {
        state.remove(&key);
    } else {
        state.insert(key, value);
    }
}
fn apply(state: BTreeMap<Vec<bool>, Amp>, g: &FlatGate) -> BTreeMap<Vec<bool>, Amp> {
    let mut out = BTreeMap::new();
    for (mut bits, a) in state {
        if !g.controls.iter().all(|(q, p)| bits[*q] == *p) {
            put(&mut out, bits, a);
            continue;
        }
        let q = g.targets[0];
        let b = bits[q];
        match g.gate {
            G::X => {
                bits[q] = !b;
                put(&mut out, bits, a);
            }
            G::Swap => {
                bits.swap(q, g.targets[1]);
                put(&mut out, bits, a);
            }
            G::Cz => {
                let a = if b && bits[g.targets[1]] {
                    a.mul(&Amp::constant(-1, 0, 0))
                } else {
                    a
                };
                put(&mut out, bits, a);
            }
            G::H => {
                bits[q] = false;
                put(&mut out, bits.clone(), a.mul(&Amp::constant(1, 1, 1)));
                bits[q] = true;
                put(
                    &mut out,
                    bits,
                    a.mul(&Amp::constant(if b { -1 } else { 1 }, 1, 1)),
                );
            }
            G::S | G::Sdg | G::T | G::Tdg => {
                let phase = match g.gate {
                    G::S => Amp::constant(1, 0, 2),
                    G::Sdg => Amp::constant(-1, 0, 2),
                    G::T => Amp::constant(1, 1, 1).add(&Amp::constant(1, 1, 3)),
                    G::Tdg => Amp::constant(1, 1, 1).add(&Amp::constant(-1, 1, 3)),
                    _ => unreachable!(),
                };
                put(&mut out, bits, if b { a.mul(&phase) } else { a });
            }
            G::Rx(t) | G::Ry(t) | G::Rz(t) => {
                let theta = D::from_f64(t).half();
                let pos = Amp::phase(theta.clone());
                let neg = Amp::phase(theta.neg());
                if matches!(g.gate, G::Rz(_)) {
                    put(&mut out, bits, a.mul(if b { &pos } else { &neg }));
                    continue;
                }
                let cos = pos.add(&neg).mul(&Amp::constant(1, 1, 0));
                put(&mut out, bits.clone(), a.mul(&cos));
                bits[q] = !b;
                let flip = if matches!(g.gate, G::Rx(_)) {
                    neg.add(&pos.mul(&Amp::constant(-1, 0, 0)))
                        .mul(&Amp::constant(1, 1, 0))
                } else {
                    pos.add(&neg.mul(&Amp::constant(-1, 0, 0)))
                        .mul(&Amp::constant(if b { 1 } else { -1 }, 1, 2))
                };
                put(&mut out, bits, a.mul(&flip));
            }
        }
    }
    out
}
pub fn basis(flat: &Flat, input: &str) -> BTreeMap<String, Amp> {
    assert_eq!(input.len(), flat.interface.len());
    let mut bits = vec![false; flat.num_qubits];
    for (i, c) in flat.interface.iter().zip(input.bytes()) {
        assert!(c == b'0' || c == b'1');
        bits[*i] = c == b'1';
    }
    // Exact basis fast path for permutation circuits: no amplitude arithmetic
    // or state-vector allocation is needed for X and SWAP permutations.
    if flat.gates.iter().all(|g| matches!(g.gate, G::X | G::Swap)) {
        for g in &flat.gates {
            if g.controls.iter().all(|(q, p)| bits[*q] == *p) {
                match g.gate {
                    G::X => bits[g.targets[0]] = !bits[g.targets[0]],
                    G::Swap => bits.swap(g.targets[0], g.targets[1]),
                    _ => unreachable!(),
                }
            }
        }
        assert!(
            flat.workspace.iter().all(|q| !bits[*q]),
            "dirty workspace after permutation"
        );
        return BTreeMap::from([(
            flat.interface
                .iter()
                .map(|q| if bits[*q] { '1' } else { '0' })
                .collect(),
            Amp::one(),
        )]);
    }
    let mut state = BTreeMap::from([(bits, Amp::one())]);
    for g in &flat.gates {
        state = apply(state, g);
    }
    let mut out = BTreeMap::new();
    for (bits, a) in state {
        assert!(
            flat.workspace.iter().all(|q| !bits[*q]),
            "dirty workspace on exact nonzero component {bits:?}: {a:?}"
        );
        let key = flat
            .interface
            .iter()
            .map(|i| if bits[*i] { '1' } else { '0' })
            .collect();
        put(&mut out, key, a);
    }
    out
}
