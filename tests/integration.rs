//! Integration suites share one binary to reduce compile/link overhead.
#[path = "check_agreement.rs"]
mod check_agreement;
#[path = "classical.rs"]
mod classical;
#[path = "diagnostics.rs"]
mod diagnostics;
#[path = "differential.rs"]
mod differential;
#[path = "exporters.rs"]
mod exporters;
#[path = "lowering.rs"]
mod lowering;
#[path = "pinned_siblings.rs"]
mod pinned_siblings;
#[path = "placement.rs"]
mod placement;
#[path = "proof_output.rs"]
mod proof_output;
#[path = "qasm.rs"]
mod qasm;
#[path = "simulator.rs"]
mod simulator;
#[path = "specialisation.rs"]
mod specialisation;
#[path = "static_checker.rs"]
mod static_checker;
#[path = "target.rs"]
mod target;
