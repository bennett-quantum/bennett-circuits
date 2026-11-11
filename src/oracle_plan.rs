//! Analysis artifacts for bounded oracle emission. No circuit is emitted here.
//! Canonical source charges/labels and the original identity survive optimization.
//!
//! Nodes contain MUST-known bits at entry, joined over all visits to the same
//! source pc and static call context. Edges/layers are MAY-reachability sets;
//! they can contain infeasible paths after joining. An absent bit/edge proof
//! never justifies deleting a dynamic check. General kernels retain their ISA
//! behavior, including memory/nested actions, rather than assuming no effects.
//! Call checks use the virtual monotonic stack counter and active depth; saved
//! callee locations and result delivery are analyzed separately for each site.
//!
//! Layer descriptors describe original row positions, including constant rows.
//! A repeated set is stored as a prefix and period, independent of B. If a long
//! period/context graph requires widening, its conservative suffix/general plan
//! is explicit. HALT is absorbing, so a later event cannot replace first fault.
//! This is an optimization artifact, not a source certificate or emitted circuit.
//! The legacy lowering remains independent and remains the production default.
mod flow;
mod known;
mod layers;
mod model;
mod transfer;
use crate::{
    Result,
    ir::Ctrl,
    object::Object,
    oracle::{Argument, Budget},
};
pub use known::KnownBits;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisOrder {
    Forward,
    FunctionsReversed,
    BlocksReversed,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub object_hash: String,
    pub closure_hash: String,
    pub oracle: String,
    pub budget: Budget,
    pub arguments: Vec<Argument>,
    pub inherited_controls: Vec<Ctrl>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RowKey {
    pub pc: usize,
    pub calls: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub pc: usize,
    pub function: String,
    pub block: String,
    pub opcode: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub function: String,
    pub name: String,
    pub width: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub min: u64,
    pub max: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Safe,
    Fault,
    Dynamic,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Row,
    Call {
        cells: u64,
        stack: Interval,
        depth: Option<u64>,
        check: Check,
    },
    Return {
        depth: Option<u64>,
    },
    Success,
    General,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    /// None is absorbing HALT. No subsequent row can override a first fault.
    pub target: Option<RowKey>,
    /// Some(None) is a fault without a small-byte code proof; None means no fault.
    pub fault: Option<Option<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub key: RowKey,
    pub values: Arc<Vec<KnownBits>>,
    pub operands: Vec<KnownBits>,
    pub successors: Arc<Vec<Edge>>,
    pub event: Event,
    /// Every node executes exactly one ORIGINAL source row, including constants.
    pub source_charge: u64,
    pub general_kernel: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layers {
    prefix: Vec<BTreeSet<RowKey>>,
    cycle: Vec<BTreeSet<RowKey>>,
    /// A conservative stable suffix replaced a long reachability period.
    pub widened: bool,
}
#[derive(Clone, Debug)]
pub struct Plan {
    binding: Binding,
    rows: Vec<Row>,
    locations: Vec<Location>,
    nodes: BTreeMap<RowKey, Node>,
    layers: Layers,
    general_reason: Option<String>,
    /// Deterministic work units, not an elapsed-time acceptance condition.
    pub analysis_visits: usize,
}
impl Plan {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn locations(&self) -> &[Location] {
        &self.locations
    }
    pub fn nodes(&self) -> &BTreeMap<RowKey, Node> {
        &self.nodes
    }
    pub fn layers(&self) -> &Layers {
        &self.layers
    }
    pub fn is_general(&self) -> bool {
        self.general_reason.is_some()
    }
    pub fn general_reason(&self) -> Option<&str> {
        self.general_reason.as_deref()
    }
    pub fn node(&self, pc: usize, calls: &[usize]) -> Option<&Node> {
        self.nodes.get(&RowKey {
            pc,
            calls: calls.to_vec(),
        })
    }
    /// Zero-based original budget position. HALT persists without a source charge.
    pub fn layer(&self, step: u64) -> &BTreeSet<RowKey> {
        self.layers.at(step)
    }
    pub fn boundary_fault(&self, position: &RowKey) -> Option<u8> {
        (position.pc != self.rows.len()).then_some(1)
    }
}
/// The analyzer accepts the same checked object/arguments/budget as the oracle.
/// Unsupported optimization proofs choose a conservative general plan, never
/// remove an export capability. Inherited controls are part of the binding.
pub fn analyze(
    obj: &Object,
    name: &str,
    budget: Budget,
    args: &[Argument],
    controls: &[Ctrl],
    order: AnalysisOrder,
) -> Result<Plan> {
    let model = model::Model::new(obj, name, budget, args)?;
    let binding = Binding {
        object_hash: obj.object_hash(),
        closure_hash: obj.identity()?[name]
            .closure_hash
            .clone()
            .ok_or_else(|| crate::Error("plan: unresolved oracle closure".into()))?,
        oracle: name.into(),
        budget,
        arguments: args.to_vec(),
        inherited_controls: controls.to_vec(),
    };
    let (nodes, general_reason, analysis_visits) = flow::solve(&model, budget, order);
    let layers = if general_reason.is_some() {
        layers::general(model.rows.len())
    } else {
        layers::build(&nodes, model.rows.len())
    };
    Ok(Plan {
        binding,
        rows: model.rows.iter().map(|r| r.public.clone()).collect(),
        locations: model.locations.clone(),
        nodes,
        layers,
        general_reason,
        analysis_visits,
    })
}
