//! Integration suites share support modules and a single linked binary.
#[path = "check_agreement.rs"]
mod check_agreement;
#[path = "classical.rs"]
mod classical;
#[path = "diagnostics.rs"]
mod diagnostics;
#[path = "differential.rs"]
mod differential;
#[path = "support/exact.rs"]
mod exact;
#[path = "export_oracle.rs"]
mod export_oracle;
#[path = "exporters.rs"]
mod exporters;
#[path = "support/fixtures.rs"]
mod fixtures;
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
#[path = "support/sibling.rs"]
mod sibling;
#[path = "support/sim.rs"]
mod sim;
#[path = "simulator.rs"]
mod simulator;
#[path = "specialisation.rs"]
mod specialisation;
#[path = "static_checker.rs"]
mod static_checker;
#[path = "target.rs"]
mod target;
#[path = "support/target.rs"]
mod test_target;
