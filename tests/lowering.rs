//! Circuit IR + Q-action lowering (brief milestone 1, deliverable 3):
//! hierarchy preserved, every §4 Q row implemented or failing with
//! `unimplemented: <row> (milestone N)`.

use std::path::{Path, PathBuf};

use bennett_circuits::flatten::flatten;
use bennett_circuits::ir::{BaseGate, Node};
use bennett_circuits::lower::lower_program;
use bennett_circuits::object::Object;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bennett-isa-spec/reference-interpreter/tests/fixtures")
}

fn fixture(name: &str) -> Object {
    let text = std::fs::read_to_string(fixtures_dir().join(name)).expect("read fixture");
    Object::parse(&text).expect("parse fixture")
}

fn action_names(obj: &Object) -> Vec<String> {
    obj.symbols
        .iter()
        .filter(|s| s.kind == "action" || s.kind == "perm_action")
        .map(|s| s.name.clone())
        .collect()
}

#[test]
fn every_fixture_action_lowers() {
    let mut total = 0;
    for entry in std::fs::read_dir(fixtures_dir()).expect("fixtures dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|x| x != "bisa") {
            continue;
        }
        let obj = Object::parse(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        for name in action_names(&obj) {
            let program = lower_program(&obj, &name)
                .unwrap_or_else(|e| panic!("{} @{name} lowering failed: {e}", path.display()));
            assert_eq!(program.top, name);
            // Flattening must also succeed with sane wire counts.
            let flat = flatten(&program)
                .unwrap_or_else(|e| panic!("{} @{name} flatten failed: {e}", path.display()));
            assert!(flat.num_qubits >= flat.interface.len());
            total += 1;
        }
    }
    assert_eq!(total, 13, "expected 13 actions across the fixtures");
}

#[test]
fn flip_keeps_uncomputation_pair() {
    let obj = fixture("actions.bisa");
    let program = lower_program(&obj, "flip").unwrap();
    let circuit = program.circuit("flip").unwrap();
    let kinds: Vec<&str> = circuit
        .body
        .iter()
        .map(|n| match n {
            Node::Copy { .. } => "copy",
            Node::Uncopy { .. } => "uncopy",
            Node::Xor { .. } => "xor",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["copy", "xor", "uncopy"]);
}

#[test]
fn calls_stay_hierarchical_and_controlled() {
    let obj = fixture("control_flow.bisa");
    let program = lower_program(&obj, "controlled_phase").unwrap();
    assert!(program.circuit("phase").is_some(), "callee circuit present");
    let top = program.circuit("controlled_phase").unwrap();
    let calls: Vec<&Node> = top
        .body
        .iter()
        .filter(|n| matches!(n, Node::Call { .. }))
        .collect();
    assert_eq!(calls.len(), 1);
    match calls[0] {
        Node::Call {
            callee,
            adjoint,
            controls,
            args,
        } => {
            assert_eq!(callee, "phase");
            assert!(!adjoint);
            assert_eq!(controls.len(), 1, "ctl p contributes one control");
            assert_eq!(args, &["t".to_string()]);
        }
        _ => unreachable!(),
    }
}

#[test]
fn qloop_unrolls_and_bell_adjoint_reverses() {
    let obj = fixture("actions.bisa");
    let program = lower_program(&obj, "loop").unwrap();
    let circuit = program.circuit("loop").unwrap();
    let xs = circuit
        .body
        .iter()
        .filter(|n| matches!(n, Node::Gate(g) if g.gate == BaseGate::X))
        .count();
    assert_eq!(xs, 3, "qloop 3 unrolls to three x gates");

    let obj = fixture("registers.bisa");
    let program = lower_program(&obj, "bell").unwrap();
    let bell = program.circuit("bell").unwrap();
    let adj = bell.adjoint();
    let gates: Vec<BaseGate> = adj
        .body
        .iter()
        .map(|n| match n {
            Node::Gate(g) => g.gate,
            other => panic!("unexpected node {other:?}"),
        })
        .collect();
    assert_eq!(
        gates,
        [BaseGate::X, BaseGate::H],
        "adjoint reverses order (cx then h)"
    );
}

fn synthetic(body: &str) -> Object {
    let text = format!(
        "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\
         \n\
         section symbols\n\
         {{\"0\":{{\"interface_hash\":\"{}\",\"kind\":\"action\",\"name\":\"a\",\"signature\":{{}}}}}}\n\
         \n\
         section functions\n\
         action @a (t:wire[2] mut) (p:wire[2] in) () () ()\n\
         ^entry():\n\
         {body}",
        "0".repeat(64)
    );
    Object::parse(&text).expect("parse synthetic")
}

#[test]
fn missing_specialisation_bindings_fail_loudly() {
    for (body, needle) in [
        ("oracle_call @o tk (p) t f () ()\n", "missing oracle token"),
        ("qinplace @g @ginv tk t\n", "missing qinplace budget token"),
        (
            "rz theta t[0]\n",
            "unimplemented: rz with non-literal angle (milestone 2)",
        ),
    ] {
        let obj = synthetic(body);
        let err = lower_program(&obj, "a").unwrap_err().to_string();
        assert!(err.contains(needle), "expected {needle:?}, got {err:?}");
    }
}

#[test]
fn non_q_opcode_and_leaks_are_errors() {
    let obj = synthetic("r = measure t\n");
    let err = lower_program(&obj, "a").unwrap_err().to_string();
    assert!(err.contains("measure"), "got {err:?}");

    let obj = synthetic("d = qcopy p\n");
    let err = lower_program(&obj, "a").unwrap_err().to_string();
    assert!(err.contains("never freed"), "got {err:?}");

    let obj = synthetic("x t[5]\n");
    assert!(lower_program(&obj, "a").is_err(), "out-of-range wire index");
}

#[test]
fn local_regions_are_lexical_entry_allocations() {
    let obj = synthetic("d = qload r p[0]\nqunload r p[0] d\nqloop 0 {\nqregion r 1 2\n}\n");
    // A one-wire bundle is required as an address; use the whole p below.
    let mut obj = obj;
    let f = obj.functions.iter_mut().find(|f| f.name == "a").unwrap();
    f.groups[1][0].ty = bennett_circuits::object::Type::Wire(1);
    for i in &mut f.blocks[0].instrs {
        for o in &mut i.operands {
            if matches!(o, bennett_circuits::object::Operand::Indexed(n, _) if n == "p") {
                *o = bennett_circuits::object::Operand::Name("p".into());
            }
        }
    }
    let program = lower_program(&obj, "a").unwrap();
    let c = program.top_circuit().unwrap();
    assert_eq!(c.local_regions.len(), 1);
    assert_eq!(
        c.workspace
            .iter()
            .filter(|w| w.kind == bennett_circuits::ir::WsKind::LocalRegion)
            .count(),
        1
    );
}

#[test]
fn cycles_and_unbalanced_arms_are_rejected() {
    let obj = synthetic("action_call @a (t) (p) () ()\n");
    assert!(
        lower_program(&obj, "a")
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    let obj = synthetic("qif p[0] {\nqpred p[1]\n} {\nqunpred p[1]\n}\n");
    assert!(lower_program(&obj, "a").is_err());
}

#[test]
fn mcx_accepts_indexed_controls() {
    let obj = synthetic("mcx (p[0], p[1]) t[0]\n");
    let flat = flatten(&lower_program(&obj, "a").unwrap()).unwrap();
    assert_eq!(flat.gates[0].controls, [(2, true), (3, true)]);
}

#[test]
fn missing_call_and_count_bindings_are_contextual_errors() {
    for (body, row) in [
        (
            "adj oracle_call @o tk (p) t f () ()\n",
            "missing oracle token",
        ),
        (
            "ctl p[0] oracle_call @o tk (p) t f () ()\n",
            "missing oracle token",
        ),
        ("qloop n {\nx t[0]\n}\n", "qloop"),
        ("d = qalloc any n\nqfree d\n", "qalloc"),
        ("d = qborrow any n\nqreturn d\n", "qborrow"),
        ("qregion r n 8\n", "qregion"),
        ("action_call @b (t) (p) () (theta)\n", "unknown action"),
    ] {
        let error = lower_program(&synthetic(body), "a")
            .unwrap_err()
            .to_string();
        assert!(error.contains(row), "{error}");
    }
}

#[test]
fn internal_loop_flags_do_not_shadow_user_names() {
    let mut obj = synthetic("qloop 2 while p {\nx t[0]\n}\ncx p[0] t[1]\n");
    // This is a valid ISA name, not a reserved implementation namespace.
    obj.functions[0].groups[0][0].name = "qloopwhile.flag1".into();
    for i in &mut obj.functions[0].blocks[0].instrs {
        fn rename(i: &mut bennett_circuits::object::Instr) {
            for o in &mut i.operands {
                if let bennett_circuits::object::Operand::Indexed(n, _) = o
                    && n == "t"
                {
                    *n = "qloopwhile.flag1".into();
                }
            }
            for child in &mut i.children {
                for i in child {
                    rename(i);
                }
            }
        }
        rename(i);
    }
    let flat = flatten(&lower_program(&obj, "a").unwrap()).unwrap();
    assert_eq!(flat.gates.last().unwrap().targets, [1]);
}
