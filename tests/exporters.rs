//! Executable-text validation of QIR and Quil, independent of the IR comments.
use super::sibling;
use super::sim;
use bennett_circuits::{flatten::flatten, ir::*, qir, quil};
fn program() -> Program {
    let q = |i| QubitRef::new("q", i);
    let mut body = vec![];
    for (i, gate) in [
        BaseGate::X,
        BaseGate::H,
        BaseGate::S,
        BaseGate::Sdg,
        BaseGate::T,
        BaseGate::Tdg,
        BaseGate::Rx(0.37),
        BaseGate::Ry(-0.23),
        BaseGate::Rz(f64::from_bits(1)),
        BaseGate::Cz,
        BaseGate::Swap,
    ]
    .into_iter()
    .enumerate()
    {
        let targets = (0..gate.arity()).map(|n| q(n as u32)).collect();
        body.push(Node::Gate(GateInst {
            own_controls: 0,
            gate,
            controls: (0..i % 4)
                .map(|n| Ctrl {
                    qubit: q(2 + n as u32),
                    positive: n % 2 == 0,
                })
                .collect(),
            targets,
        }));
    }
    let circuit = Circuit {
        name: "all gates".into(),
        perm: false,
        mut_bundles: vec![Bundle {
            name: "q".into(),
            width: 5,
            role: Role::Mut,
        }],
        in_bundles: vec![],
        mem: vec![],
        workspace: vec![],
        local_regions: vec![],
        body,
    };
    Program {
        top: circuit.name.clone(),
        circuits: vec![circuit],
    }
}
fn same_states(a: &bennett_circuits::flatten::Flat, b: &bennett_circuits::flatten::Flat) {
    for n in 0..1usize << a.interface.len() {
        let bits = (0..a.interface.len())
            .map(|i| if n >> i & 1 == 0 { '0' } else { '1' })
            .collect::<String>();
        assert_eq!(sim::basis(a, &bits), sim::basis(b, &bits), "input={bits}");
    }
}
#[test]
fn all_gates_controls_polarities_and_subnormal_angles_roundtrip() {
    let p = program();
    let original = flatten(&p).unwrap();
    let q = qir::export(&p).unwrap();
    assert_eq!(qir::parse(&q).unwrap(), p);
    same_states(&original, &qir::parse_flat(&q).unwrap());
    let q = quil::export(&p).unwrap();
    assert_eq!(quil::parse(&q).unwrap(), p);
    same_states(&original, &quil::parse_flat(&q).unwrap());
}
#[test]
fn executable_mutations_are_seen_without_ir_metadata() {
    let p = program();
    let text = qir::export(&p).unwrap();
    let bad = text.replacen("@__quantum__qis__x__body", "@__quantum__qis__h__body", 1);
    assert!(qir::parse(&bad).is_err());
    assert_ne!(
        qir::parse_flat(&bad).unwrap().gates,
        qir::parse_flat(&text).unwrap().gates
    );
    let executable = text
        .lines()
        .filter(|l| !l.starts_with("; bc: "))
        .collect::<Vec<_>>()
        .join("\n");
    same_states(
        &flatten(&p).unwrap(),
        &qir::parse_flat(&executable).unwrap(),
    );
    let text = quil::export(&p).unwrap();
    let bad = text.replacen("X 0", "H 0", 1);
    assert!(quil::parse(&bad).is_err());
    assert_ne!(
        quil::parse_flat(&bad).unwrap().gates,
        quil::parse_flat(&text).unwrap().gates
    );
    let executable = text
        .lines()
        .filter(|l| !l.starts_with("# bc: "))
        .collect::<Vec<_>>()
        .join("\n");
    same_states(
        &flatten(&p).unwrap(),
        &quil::parse_flat(&executable).unwrap(),
    );
}
#[test]
fn external_llvm_assembler_and_verifier_accept_qir() {
    use std::process::Command;
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/sol-circuits-m5/validators");
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("all.ll");
    let output = root.join("all.bc");
    std::fs::write(&input, qir::export(&program()).unwrap()).unwrap();
    for (tool, args) in [
        (
            "llvm-as-18",
            vec![input.to_str().unwrap(), "-o", output.to_str().unwrap()],
        ),
        (
            "opt-18",
            vec![
                "-passes=verify",
                "-disable-output",
                output.to_str().unwrap(),
            ],
        ),
    ] {
        match Command::new(tool).args(args).output() {
            Ok(o) => assert!(
                o.status.success(),
                "{tool}: {}",
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("external validator unavailable: {tool}");
                return;
            }
            Err(e) => panic!("{tool}: {e}"),
        }
    }
}
#[test]
fn zero_wire_identity_is_valid_without_a_sentinel() {
    let p = Program {
        top: "identity".into(),
        circuits: vec![Circuit {
            name: "identity".into(),
            perm: true,
            mut_bundles: vec![],
            in_bundles: vec![],
            mem: vec![],
            workspace: vec![],
            local_regions: vec![],
            body: vec![],
        }],
    };
    assert_eq!(qir::parse(&qir::export(&p).unwrap()).unwrap(), p);
    assert_eq!(quil::parse(&quil::export(&p).unwrap()).unwrap(), p);
}
#[test]
fn cli_exports_actions_tokens_and_bounded_oracles() {
    use std::process::Command;
    let fixtures = sibling::isa().join("reference-interpreter/tests/fixtures");
    for (fixture, name, options) in [
        ("control_flow.bisa", "controlled_phase", vec![]),
        ("r9-sol-oracle.bisa", "a", vec!["--arg", "token:1,0,0,0"]),
        ("r9-sol-oracle.bisa", "o", vec!["--budget", "1,0,0,0"]),
    ] {
        let path = fixtures.join(fixture);
        let qasm = Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
            .args(["export-qasm", path.to_str().unwrap(), name])
            .args(&options)
            .output()
            .unwrap();
        assert!(
            qasm.status.success(),
            "{}",
            String::from_utf8_lossy(&qasm.stderr)
        );
        let source =
            bennett_circuits::qasm::parse(std::str::from_utf8(&qasm.stdout).unwrap()).unwrap();
        for format in ["qir", "quil"] {
            let output = Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
                .args([&format!("export-{format}"), path.to_str().unwrap(), name])
                .args(&options)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = std::str::from_utf8(&output.stdout).unwrap();
            let (p, f) = if format == "qir" {
                (qir::parse(text).unwrap(), qir::parse_flat(text).unwrap())
            } else {
                (quil::parse(text).unwrap(), quil::parse_flat(text).unwrap())
            };
            assert_eq!(p, source);
            same_states(&flatten(&source).unwrap(), &f);
        }
    }
}
#[test]
fn every_gate_every_control_polarity_pattern_has_the_same_exact_state() {
    let source = program();
    let mut gates = source.circuits[0]
        .body
        .iter()
        .map(|n| match n {
            Node::Gate(g) => g.gate,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    gates.push(BaseGate::Rz(-0.0));
    for gate in gates {
        for controls in 0..=3 {
            for polarity in 0..1usize << controls {
                let width = gate.arity() + controls;
                let mut p = source.clone();
                let c = &mut p.circuits[0];
                c.mut_bundles[0].width = width as u32;
                c.body = vec![Node::Gate(GateInst {
                    own_controls: controls,
                    gate,
                    controls: (0..controls)
                        .map(|i| Ctrl {
                            qubit: QubitRef::new("q", (gate.arity() + i) as u32),
                            positive: polarity >> i & 1 != 0,
                        })
                        .collect(),
                    targets: (0..gate.arity())
                        .map(|i| QubitRef::new("q", i as u32))
                        .collect(),
                })];
                let original = flatten(&p).unwrap();
                for (name, text) in [
                    ("QIR", qir::export(&p).unwrap()),
                    ("Quil", quil::export(&p).unwrap()),
                ] {
                    let parsed = if name == "QIR" {
                        qir::parse_flat(&text).unwrap()
                    } else {
                        quil::parse_flat(&text).unwrap()
                    };
                    assert_eq!(original.num_qubits, parsed.num_qubits, "{name}: {gate:?}");
                    same_states(&original, &parsed);
                }
            }
        }
    }
}
