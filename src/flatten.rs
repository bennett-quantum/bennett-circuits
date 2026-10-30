//! Naive static placement (milestone 1): hierarchical IR → flat gate
//! list over allocated qubit indices. Interface wires come first, in
//! `run-action` order (mut bundles, in bundles, mem cells; bundle bit
//! `i` at offset `i`); every workspace bundle of every call instance
//! gets fresh wires. Real placement (liveness reuse, classes, target
//! schemes) is milestone 3.

use std::collections::BTreeMap;

use crate::ir::*;
use crate::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub struct FlatGate {
    pub gate: BaseGate,
    /// (qubit index, positive polarity)
    pub controls: Vec<(usize, bool)>,
    pub targets: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flat {
    pub num_qubits: usize,
    /// Qubit indices of the top circuit's interface, in interface order.
    pub interface: Vec<usize>,
    /// Every non-interface qubit (must end clean / borrowed-restored).
    pub workspace: Vec<usize>,
    pub gates: Vec<FlatGate>,
}

pub fn flatten(program: &Program) -> Result<Flat> {
    Ok(flatten_with_events(program)?.0)
}
/// Static §6.2 events at gate boundaries. A call's declared contract
/// closes windows even if its implementation consists of diagonal gates.
pub(crate) type CallEvents = Vec<(usize, Vec<usize>)>;
pub(crate) fn flatten_with_events(program: &Program) -> Result<(Flat, CallEvents)> {
    let (flat, events, _) = flatten_allocations(program)?;
    Ok((flat, events))
}

/// Allocation provenance for placement, including nested and adjoint instances.
#[derive(Debug, Clone)]
pub(crate) struct Allocation {
    pub wires: Vec<usize>,
    pub kind: WsKind,
    pub class: String,
}
pub(crate) fn flatten_allocations(
    program: &Program,
) -> Result<(Flat, CallEvents, Vec<Allocation>)> {
    let top = program.top_circuit()?;
    let mut alloc = Allocator {
        next: 0,
        workspace: Vec::new(),
        nonperm_calls: Vec::new(),
        allocations: Vec::new(),
    };
    let mut map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut interface = Vec::new();
    for (name, width) in top.interface_bundles() {
        let wires = alloc.fresh(width, false);
        interface.extend_from_slice(&wires);
        map.insert(name, wires);
    }
    let mut gates = Vec::new();
    expand(top, program, &mut alloc, &map, &[], &mut gates, 0)?;
    Ok((
        Flat {
            num_qubits: alloc.next,
            interface,
            workspace: alloc.workspace,
            gates,
        },
        alloc.nonperm_calls,
        alloc.allocations,
    ))
}

struct Allocator {
    allocations: Vec<Allocation>,
    next: usize,
    workspace: Vec<usize>,
    nonperm_calls: Vec<(usize, Vec<usize>)>,
}

impl Allocator {
    fn fresh(&mut self, width: u64, is_workspace: bool) -> Vec<usize> {
        let start = self.next;
        self.next += width as usize;
        let wires: Vec<usize> = (start..self.next).collect();
        if is_workspace {
            self.workspace.extend_from_slice(&wires);
        }
        wires
    }
}

fn q(map: &BTreeMap<String, Vec<usize>>, r: &QubitRef) -> Result<usize> {
    map.get(&r.bundle)
        .and_then(|w| w.get(r.index as usize))
        .copied()
        .ok_or_else(|| crate::Error(format!("flatten: unmapped wire {r:?}")))
}

fn ctrls(
    map: &BTreeMap<String, Vec<usize>>,
    inherited: &[(usize, bool)],
    own: &[Ctrl],
) -> Result<Vec<(usize, bool)>> {
    let mut out: Vec<(usize, bool)> = inherited.to_vec();
    for c in own {
        out.push((q(map, &c.qubit)?, c.positive));
    }
    Ok(out)
}

fn bundle<'m>(map: &'m BTreeMap<String, Vec<usize>>, name: &str) -> Result<&'m Vec<usize>> {
    map.get(name)
        .ok_or_else(|| crate::Error(format!("flatten: unmapped bundle {name:?}")))
}

fn expand(
    circuit: &Circuit,
    program: &Program,
    alloc: &mut Allocator,
    outer_map: &BTreeMap<String, Vec<usize>>,
    inherited: &[(usize, bool)],
    gates: &mut Vec<FlatGate>,
    depth: usize,
) -> Result<()> {
    if depth > 512 {
        bail!("flatten: call depth over 512 (host limit)");
    }
    // Workspace of this instance.
    let mut map = outer_map.clone();
    for w in &circuit.workspace {
        let wires = alloc.fresh(u64::from(w.width), true);
        alloc.allocations.push(Allocation {
            wires: wires.clone(),
            kind: w.kind,
            class: w.class.clone(),
        });
        map.insert(w.name.clone(), wires);
    }
    for node in &circuit.body {
        match node {
            Node::Gate(g) => {
                gates.push(FlatGate {
                    gate: g.gate,
                    controls: ctrls(&map, inherited, &g.controls)?,
                    targets: g
                        .targets
                        .iter()
                        .map(|t| q(&map, t))
                        .collect::<Result<Vec<_>>>()?,
                });
            }
            Node::Copy { src, dst, controls } | Node::Uncopy { src, dst, controls } => {
                let (s, d) = (bundle(&map, src)?.clone(), bundle(&map, dst)?.clone());
                if s.len() != d.len() {
                    bail!("flatten: qcopy width mismatch");
                }
                let base = ctrls(&map, inherited, controls)?;
                for (si, di) in s.iter().zip(&d) {
                    let mut cs = base.clone();
                    cs.push((*si, true));
                    gates.push(FlatGate {
                        gate: BaseGate::X,
                        controls: cs,
                        targets: vec![*di],
                    });
                }
            }
            Node::Xor { src, tgt, controls } => {
                let (s, d) = (bundle(&map, src)?.clone(), bundle(&map, tgt)?.clone());
                let base = ctrls(&map, inherited, controls)?;
                for (si, di) in s.iter().zip(&d) {
                    let mut cs = base.clone();
                    cs.push((*si, true));
                    gates.push(FlatGate {
                        gate: BaseGate::X,
                        controls: cs,
                        targets: vec![*di],
                    });
                }
            }
            Node::Exch { a, b, controls } => {
                let (x, y) = (bundle(&map, a)?.clone(), bundle(&map, b)?.clone());
                let base = ctrls(&map, inherited, controls)?;
                for (xi, yi) in x.iter().zip(&y) {
                    gates.push(FlatGate {
                        gate: BaseGate::Swap,
                        controls: base.clone(),
                        targets: vec![*xi, *yi],
                    });
                }
            }
            Node::Load {
                region,
                addr,
                dst,
                controls,
            }
            | Node::Unload {
                region,
                addr,
                dst,
                controls,
            } => {
                expand_mem(
                    circuit,
                    &map,
                    inherited,
                    controls,
                    region,
                    addr,
                    dst,
                    MemOp::Load,
                    gates,
                )?;
            }
            Node::Store {
                region,
                addr,
                val,
                controls,
            } => {
                expand_mem(
                    circuit,
                    &map,
                    inherited,
                    controls,
                    region,
                    addr,
                    val,
                    MemOp::Store,
                    gates,
                )?;
            }
            Node::ExchM {
                region,
                addr,
                val,
                controls,
            } => {
                expand_mem(
                    circuit,
                    &map,
                    inherited,
                    controls,
                    region,
                    addr,
                    val,
                    MemOp::Exch,
                    gates,
                )?;
            }
            Node::Call {
                callee,
                adjoint,
                controls,
                args,
            } => {
                let Some(target) = program.circuit(callee) else {
                    bail!("flatten: call to missing circuit {callee:?}");
                };
                let target = if *adjoint {
                    target.adjoint()
                } else {
                    target.clone()
                };
                let formals = target.interface_bundles();
                if formals.len() != args.len() {
                    bail!("flatten: call @{callee} arity mismatch");
                }
                let mut call_map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
                for ((formal, fw), actual) in formals.iter().zip(args) {
                    let wires = bundle(&map, actual)?.clone();
                    if wires.len() as u64 != *fw {
                        bail!("flatten: call @{callee} width mismatch on {actual:?}");
                    }
                    call_map.insert(formal.clone(), wires);
                }
                let mut inh = ctrls(&map, inherited, controls)?;
                if !target.perm {
                    let touched = call_map.values().flatten().copied().collect();
                    alloc.nonperm_calls.push((gates.len(), touched));
                }
                expand(&target, program, alloc, &call_map, &inh, gates, depth + 1)?;
                inh.clear();
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum MemOp {
    Load,
    Store,
    Exch,
}

/// §4.6 address dispatch: for each cell, under `addr == i`.
#[allow(clippy::too_many_arguments)]
fn expand_mem(
    circuit: &Circuit,
    map: &BTreeMap<String, Vec<usize>>,
    inherited: &[(usize, bool)],
    controls: &[Ctrl],
    region: &str,
    addr: &str,
    val: &str,
    op: MemOp,
    gates: &mut Vec<FlatGate>,
) -> Result<()> {
    let Some((a_w, cell_w)) = circuit.region_geom(region) else {
        bail!("flatten: unknown region {region:?}");
    };
    let cells = bundle(map, region)?.clone();
    let addr_wires = bundle(map, addr)?.clone();
    let val_wires = bundle(map, val)?.clone();
    if addr_wires.len() != a_w as usize || val_wires.len() != cell_w as usize {
        bail!("flatten: qmem width mismatch on {region:?}");
    }
    let base = ctrls(map, inherited, controls)?;
    let num_cells = 1u64 << a_w;
    for cell in 0..num_cells {
        let mut eq = base.clone();
        for (b, wire) in addr_wires.iter().enumerate() {
            eq.push((*wire, (cell >> b) & 1 == 1));
        }
        for j in 0..cell_w as usize {
            let cell_wire = cells[cell as usize * cell_w as usize + j];
            match op {
                MemOp::Load => {
                    let mut cs = eq.clone();
                    cs.push((cell_wire, true));
                    gates.push(FlatGate {
                        gate: BaseGate::X,
                        controls: cs,
                        targets: vec![val_wires[j]],
                    });
                }
                MemOp::Store => {
                    let mut cs = eq.clone();
                    cs.push((val_wires[j], true));
                    gates.push(FlatGate {
                        gate: BaseGate::X,
                        controls: cs,
                        targets: vec![cell_wire],
                    });
                }
                MemOp::Exch => {
                    gates.push(FlatGate {
                        gate: BaseGate::Swap,
                        controls: eq.clone(),
                        targets: vec![val_wires[j], cell_wire],
                    });
                }
            }
        }
    }
    Ok(())
}
