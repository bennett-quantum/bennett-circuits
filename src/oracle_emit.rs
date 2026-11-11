//! Replayable reversible source-row recipes. The literal lowering is independent.
//! Each layer retains overwritten values and all arithmetic scratch until the
//! exact inverse prefix. No measurement or non-injective arithmetic inverse.
//! Allocation/count summaries follow the analyzed prefix/period; reverse replay
//! reconstructs the same numeric wire IDs without retaining the forward gates.
mod bits;
pub(crate) mod dispatch;
mod machine;
mod replay;
mod source;
use crate::{
    Error, Result,
    ir::*,
    object::Object,
    oracle::{Argument, Budget},
    oracle_plan::{self, AnalysisOrder, KnownBits, Plan, RowKey},
};
use bits::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flip {
    pub target: usize,
    pub controls: Vec<(usize, bool)>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kernels {
    pub constant_multiplications: u128,
    pub generic_multiplications: u128,
    pub fixed_shifts: u128,
    pub variable_shifts: u128,
    pub stack_scans: u128,
    pub equalities: u128,
    pub ordered_comparisons: u128,
}
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub forward: u128,
    pub commit: u128,
    pub full: u128,
    pub histogram: BTreeMap<usize, u128>,
    pub max_controls: usize,
    pub qubits: u128,
    pub history_wires: u128,
    pub peak_gate_objects: usize,
    pub layer_descriptors: usize,
    pub source_row_contexts: u128,
    pub kernels: Kernels,
}
#[derive(Clone, Debug, Default)]
struct Layout {
    alloc: usize,
    gates: usize,
    hist: BTreeMap<usize, u128>,
    kernels: Kernels,
}
/// Validated immutable recipes. `visit` is the bounded-resident gate API.
/// `to_program` is a compatibility adapter which materializes an IR for callers
/// that require one; a streaming writer can consume `visit` directly.
pub struct Recipes {
    plan: Plan,
    source: source::Source,
    machine: machine::Machine,
    prefix: Vec<Layout>,
    cycle: Vec<Layout>,
    commit: Layout,
    counts: Counts,
}
impl Recipes {
    pub fn counts(&self) -> &Counts {
        &self.counts
    }
    pub fn plan(&self) -> &Plan {
        &self.plan
    }
    pub fn interface_wires(&self) -> usize {
        self.source.interface.len()
    }
    pub fn wire_count(&self) -> Result<usize> {
        usize::try_from(self.counts.qubits)
            .map_err(|_| Error("host limit: recipe wire count".into()))
    }
    pub fn forward_gates_at(&self, step: u64) -> usize {
        self.layout(step).gates
    }
}
/// None selects the complete literal bounded machine for an unproved plan;
/// unsupported optimization never rejects an otherwise exportable oracle.
pub fn prepare(
    obj: &Object,
    name: &str,
    budget: Budget,
    args: &[Argument],
) -> Result<Option<Recipes>> {
    let plan = oracle_plan::analyze(obj, name, budget, args, &[], AnalysisOrder::Forward)?;
    if !source::eligible(&plan) || !obj.function(name).unwrap().groups[3].is_empty() {
        return Ok(None);
    }
    let source = source::Source::new(obj, &plan);
    let machine = machine::Machine::new(&plan, &source);
    let mut recipe = Recipes {
        plan,
        source,
        machine,
        prefix: vec![],
        cycle: vec![],
        commit: Layout::default(),
        counts: Counts::default(),
    };
    recipe.summarize()?;
    Ok(Some(recipe))
}
