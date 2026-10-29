//! The hierarchical circuit IR. Lowering produces it; flattening,
//! exporters and (later) placement/scheduling/certificate generation
//! consume it. Calls stay calls and uncomputation pairs stay explicit
//! (docs/PLAN.md). Predication lives in gate/node control lists (§3.5);
//! bounded loops are already unrolled by lowering.

use crate::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Mut,
    In,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub name: String,
    pub width: u32,
    pub role: Role,
}

/// An interface quantum-memory region (`region[A, W]`), flattened to
/// `2^A * W` wires in cell order, cell `i` at wires `[i*W, (i+1)*W)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemBundle {
    pub name: String,
    pub addr_width: u32,
    pub cell_width: u32,
    pub role: Role,
}

impl MemBundle {
    pub fn num_cells(&self) -> u64 {
        1u64 << self.addr_width
    }
    pub fn width(&self) -> u64 {
        self.num_cells() * u64::from(self.cell_width)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsKind {
    /// `qalloc`, `qcopy`/`qload` products, loop flags: start and end |0⟩.
    Clean,
    /// `qborrow`: unknown entry state; the circuit must act as identity
    /// on it overall (§5.6). Export/simulation instantiate the pool in
    /// |0⟩ (matching `run-action`).
    Borrowed,
    /// Cells of a function-local `qregion` (one flattened bundle).
    LocalRegion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub name: String,
    pub width: u32,
    pub kind: WsKind,
    /// Allocation class as written (`any` or a target class name);
    /// placement resolves it in milestone 3.
    pub class: String,
}

/// Geometry of a function-local region; its cells are the workspace
/// bundle of the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionInfo {
    pub name: String,
    pub addr_width: u32,
    pub cell_width: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct QubitRef {
    pub bundle: String,
    pub index: u32,
}

impl QubitRef {
    pub fn new(bundle: &str, index: u32) -> QubitRef {
        QubitRef {
            bundle: bundle.to_string(),
            index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ctrl {
    pub qubit: QubitRef,
    pub positive: bool,
}

impl Ctrl {
    pub fn pos(qubit: QubitRef) -> Ctrl {
        Ctrl {
            qubit,
            positive: true,
        }
    }
}

/// §4.4 base gates. `cx`/`ccx`/`mcx(k)` are `X` with 1/2/k controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BaseGate {
    X,
    H,
    S,
    Sdg,
    T,
    Tdg,
    Cz,
    Swap,
    Rx(f64),
    Ry(f64),
    Rz(f64),
}

impl BaseGate {
    pub fn adjoint(self) -> BaseGate {
        match self {
            BaseGate::S => BaseGate::Sdg,
            BaseGate::Sdg => BaseGate::S,
            BaseGate::T => BaseGate::Tdg,
            BaseGate::Tdg => BaseGate::T,
            BaseGate::Rx(t) => BaseGate::Rx(-t),
            BaseGate::Ry(t) => BaseGate::Ry(-t),
            BaseGate::Rz(t) => BaseGate::Rz(-t),
            g => g,
        }
    }

    /// Target count (controls excluded).
    pub fn arity(self) -> usize {
        match self {
            BaseGate::Cz | BaseGate::Swap => 2,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateInst {
    pub gate: BaseGate,
    pub controls: Vec<Ctrl>,
    pub targets: Vec<QubitRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Gate(GateInst),
    /// `qcopy`: `dst` (fresh, clean) ^= `src`, bit by bit.
    Copy {
        src: String,
        dst: String,
        controls: Vec<Ctrl>,
    },
    /// `quncopy`: inverse of its `Copy`; consumes `dst`.
    Uncopy {
        src: String,
        dst: String,
        controls: Vec<Ctrl>,
    },
    /// `qxor`: `tgt` ^= `src`.
    Xor {
        src: String,
        tgt: String,
        controls: Vec<Ctrl>,
    },
    /// `qexch`: bundle-wise swap.
    Exch {
        a: String,
        b: String,
        controls: Vec<Ctrl>,
    },
    /// `qload`: `dst` (fresh, clean, cell width) ^= `region[addr]`.
    Load {
        region: String,
        addr: String,
        dst: String,
        controls: Vec<Ctrl>,
    },
    /// `qunload`: inverse of its `Load`; consumes `dst`.
    Unload {
        region: String,
        addr: String,
        dst: String,
        controls: Vec<Ctrl>,
    },
    /// `qstore`: `region[addr]` ^= `val`.
    Store {
        region: String,
        addr: String,
        val: String,
        controls: Vec<Ctrl>,
    },
    /// `qexchm`: swap `val` with `region[addr]`.
    ExchM {
        region: String,
        addr: String,
        val: String,
        controls: Vec<Ctrl>,
    },
    /// A region call kept hierarchical. `args` are caller bundle names
    /// in callee interface order (mut, in, mem). `controls` are the
    /// `ctl`/predicate controls of the call site.
    Call {
        callee: String,
        adjoint: bool,
        controls: Vec<Ctrl>,
        args: Vec<String>,
    },
}

impl Node {
    pub fn adjoint(&self) -> Node {
        match self {
            Node::Gate(g) => Node::Gate(GateInst {
                gate: g.gate.adjoint(),
                controls: g.controls.clone(),
                targets: g.targets.clone(),
            }),
            Node::Copy { src, dst, controls } => Node::Uncopy {
                src: src.clone(),
                dst: dst.clone(),
                controls: controls.clone(),
            },
            Node::Uncopy { src, dst, controls } => Node::Copy {
                src: src.clone(),
                dst: dst.clone(),
                controls: controls.clone(),
            },
            Node::Load {
                region,
                addr,
                dst,
                controls,
            } => Node::Unload {
                region: region.clone(),
                addr: addr.clone(),
                dst: dst.clone(),
                controls: controls.clone(),
            },
            Node::Unload {
                region,
                addr,
                dst,
                controls,
            } => Node::Load {
                region: region.clone(),
                addr: addr.clone(),
                dst: dst.clone(),
                controls: controls.clone(),
            },
            Node::Call {
                callee,
                adjoint,
                controls,
                args,
            } => Node::Call {
                callee: callee.clone(),
                adjoint: !adjoint,
                controls: controls.clone(),
                args: args.clone(),
            },
            // Self-adjoint bundle ops.
            other => other.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Circuit {
    pub name: String,
    /// True for `perm_action` (permutation matrix, ε = 0, §5.3).
    pub perm: bool,
    pub mut_bundles: Vec<Bundle>,
    pub in_bundles: Vec<Bundle>,
    pub mem: Vec<MemBundle>,
    pub workspace: Vec<Workspace>,
    pub local_regions: Vec<RegionInfo>,
    pub body: Vec<Node>,
}

impl Circuit {
    /// Interface bundles in call order: mut bundles, in bundles, mem.
    pub fn interface_bundles(&self) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        for b in &self.mut_bundles {
            out.push((b.name.clone(), u64::from(b.width)));
        }
        for b in &self.in_bundles {
            out.push((b.name.clone(), u64::from(b.width)));
        }
        for m in &self.mem {
            out.push((m.name.clone(), m.width()));
        }
        out
    }

    pub fn interface_wire_count(&self) -> u64 {
        self.interface_bundles().iter().map(|(_, w)| w).sum()
    }

    pub fn bundle_width(&self, name: &str) -> Option<u64> {
        for b in self.mut_bundles.iter().chain(&self.in_bundles) {
            if b.name == name {
                return Some(u64::from(b.width));
            }
        }
        for m in &self.mem {
            if m.name == name {
                return Some(m.width());
            }
        }
        for w in &self.workspace {
            if w.name == name {
                return Some(u64::from(w.width));
            }
        }
        None
    }

    /// (addr width, cell width) for a region name, local or interface.
    pub fn region_geom(&self, name: &str) -> Option<(u32, u32)> {
        for m in &self.mem {
            if m.name == name {
                return Some((m.addr_width, m.cell_width));
            }
        }
        for r in &self.local_regions {
            if r.name == name {
                return Some((r.addr_width, r.cell_width));
            }
        }
        None
    }

    /// The §5.3 adjoint as a circuit transform (same interface/workspace,
    /// reversed adjoint body).
    pub fn adjoint(&self) -> Circuit {
        let mut c = self.clone();
        c.body = self.body.iter().rev().map(Node::adjoint).collect();
        c
    }

    pub fn check_widths(&self) -> Result<()> {
        for (name, w) in self.interface_bundles() {
            if w > u64::from(u32::MAX) {
                bail!(
                    "ir: bundle {name:?} of circuit {} is too wide ({w} wires)",
                    self.name
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    /// Top circuit first, followed by its reachable callees.
    pub circuits: Vec<Circuit>,
    pub top: String,
}

impl Program {
    pub fn circuit(&self, name: &str) -> Option<&Circuit> {
        self.circuits.iter().find(|c| c.name == name)
    }

    pub fn top_circuit(&self) -> Result<&Circuit> {
        self.circuit(&self.top)
            .ok_or_else(|| crate::Error(format!("ir: top circuit {} missing", self.top)))
    }
}
