#[path = "support/exact.rs"]
mod exact;
#[path = "support/sim.rs"]
mod sim;
use bennett_circuits::{
    flatten::{Flat, FlatGate},
    ir::BaseGate as G,
};
fn circuit(gates: Vec<G>) -> Flat {
    Flat {
        num_qubits: 1,
        interface: vec![0],
        workspace: vec![],
        gates: gates
            .into_iter()
            .map(|gate| FlatGate {
                gate,
                controls: vec![],
                targets: vec![0],
            })
            .collect(),
    }
}
#[test]
fn exact_cancellation_and_tiny_support() {
    for gates in [
        vec![G::H, G::H],
        vec![G::T, G::Tdg],
        vec![G::Rx(0.31), G::Rx(-0.31)],
        vec![G::Ry(0.31), G::Ry(-0.31)],
        vec![G::H, G::S, G::Sdg, G::H],
    ] {
        let state = sim::basis(&circuit(gates), "0");
        assert_eq!(state.len(), 1);
        assert_eq!(state["0"], exact::Amp::one());
    }
    let state = sim::basis(&circuit(vec![G::Ry(1e-30)]), "0");
    assert_eq!(state.len(), 2, "tiny amplitude must remain exactly nonzero");
}
#[test]
fn matrices_and_relative_global_phase() {
    let state = sim::basis(&circuit(vec![G::H]), "1");
    assert!((state["0"].value().0 - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-15);
    assert!((state["1"].value().0 + std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-15);
    let state = sim::basis(&circuit(vec![G::Rz(std::f64::consts::TAU)]), "0");
    assert!((state["0"].value().0 + 1.0).abs() < 1e-15);
}
#[test]
#[should_panic(expected = "workspace")]
fn dirty_workspace_is_a_failure() {
    sim::basis(
        &Flat {
            num_qubits: 2,
            interface: vec![0],
            workspace: vec![1],
            gates: vec![FlatGate {
                gate: G::X,
                controls: vec![],
                targets: vec![1],
            }],
        },
        "0",
    );
}
