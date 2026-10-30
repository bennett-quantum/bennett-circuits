#[path = "support/exact.rs"]
mod exact;
#[path = "support/sim.rs"]
mod sim;
use bennett_circuits::{
    flatten::{Flat, flatten},
    json::Value,
    lower::lower_program,
    object::{Object, Symbol, derived_signature},
    placement::place_action,
};

fn object(body: &str) -> Object {
    let mut o = Object::parse(&format!("bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\n{body}")).unwrap();
    o.symbols = o
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| Symbol {
            index: i.to_string(),
            name: f.name.clone(),
            kind: f.kind.symbol_kind().into(),
            signature: derived_signature(f).unwrap(),
            stored_interface_hash: "0".repeat(64),
        })
        .collect();
    let h = o.hash().unwrap();
    for s in &mut o.symbols {
        s.stored_interface_hash = h.symbols[&s.name].interface_hash.clone();
    }
    o.check().unwrap();
    o
}
fn equivalent(original: &Flat, placed: &Flat) -> usize {
    assert_eq!(original.interface, placed.interface);
    let n = original.interface.len();
    assert!(n < 16);
    for basis in 0..1usize << n {
        let input = (0..n)
            .map(|i| if basis >> i & 1 == 0 { '0' } else { '1' })
            .collect::<String>();
        assert_eq!(
            sim::basis(original, &input),
            sim::basis(placed, &input),
            "input={input}"
        );
    }
    1 << n
}
#[test]
fn sequential_clean_lifetimes_reuse_one_slot_with_known_phase() {
    let o = object(
        "action @a (t:wire mut) () () () ()\n^entry():\nd = qcopy t\ns d\nquncopy t d\ne = qcopy t\nt e\nquncopy t e\n",
    );
    let original = flatten(&lower_program(&o, "a").unwrap()).unwrap();
    let p = place_action(&o, "a", &[]).unwrap();
    assert_eq!(original.num_qubits, 3);
    assert_eq!(p.flat.num_qubits, 2);
    assert_eq!(p.logical_to_physical, vec![Some(0), Some(1), Some(1)]);
    equivalent(&original, &p.flat);
    let state = sim::basis(&p.flat, "1");
    let expected = exact::Amp::constant(1, 0, 2)
        .mul(&exact::Amp::constant(1, 1, 1).add(&exact::Amp::constant(1, 1, 3)));
    assert_eq!(
        state["1"], expected,
        "hand-derived phase is i * exp(i*pi/4)"
    );
    let (re, im) = state["1"].value();
    assert!((re + std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-15);
    assert!((im - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-15);
    let twice = place_action(&o, "a", &[]).unwrap();
    assert_eq!(p, twice, "placement must be deterministic");
}
#[test]
fn overlapping_and_control_only_uses_cannot_alias() {
    let o = object(
        "perm_action @a (t:wire mut) (x:wire in) () () ()\n^entry():\nd = qcopy x\ne = qcopy x\nccx d e t\nquncopy x d\nquncopy x e\n",
    );
    let p = place_action(&o, "a", &[]).unwrap();
    assert_eq!(p.flat.num_qubits, 4);
    assert_ne!(p.logical_to_physical[2], p.logical_to_physical[3]);
    equivalent(&flatten(&lower_program(&o, "a").unwrap()).unwrap(), &p.flat);
}
#[test]
fn requested_classes_are_separate_and_unused_clean_wires_disappear() {
    let o = object(
        "perm_action @a (t:wire mut) () () () ()\n^entry():\nd = qalloc red 1\nx d\nx d\nqfree d\ne = qalloc blue 1\nx e\nx e\nqfree e\nf = qalloc red 1\nx f\nx f\nqfree f\nu = qalloc any 1\nqfree u\n",
    );
    let p = place_action(&o, "a", &[]).unwrap();
    assert_eq!(p.flat.num_qubits, 3);
    assert_eq!(
        p.logical_to_physical,
        vec![Some(0), Some(1), Some(2), Some(1), None]
    );
    equivalent(&flatten(&lower_program(&o, "a").unwrap()).unwrap(), &p.flat);
}
#[test]
fn controlled_callee_instances_reuse_clean_scratch_preserving_interference() {
    let o = object(
        "action @sub (t:wire mut) () () () ()\n^entry():\nd = qcopy t\ns d\nquncopy t d\naction @a (t:wire mut, p:wire mut) () () () ()\n^entry():\nh p\nctl p action_call @sub (t) () () ()\nctl p adj action_call @sub (t) () () ()\nh p\n",
    );
    let original = flatten(&lower_program(&o, "a").unwrap()).unwrap();
    let p = place_action(&o, "a", &[]).unwrap();
    assert_eq!((original.num_qubits, p.flat.num_qubits), (4, 3));
    equivalent(&original, &p.flat);
    for input in ["00", "01", "10", "11"] {
        assert_eq!(
            sim::basis(&p.flat, input),
            std::collections::BTreeMap::from([(input.to_string(), exact::Amp::one())])
        );
    }
}
#[test]
fn borrowed_wires_keep_dedicated_slots_including_arbitrary_dirty_inputs() {
    let o = object(
        "perm_action @sub (t:wire mut) (x:wire in) () () ()\n^entry():\nb = qborrow any 1\ncx x b\ncx b t\ncx x b\ncx b t\nqreturn b\nperm_action @a (t:wire mut) (x:wire in) () () ()\n^entry():\naction_call @sub (t) (x) () ()\naction_call @sub (t) (x) () ()\n",
    );
    let mut original = flatten(&lower_program(&o, "a").unwrap()).unwrap();
    let p = place_action(&o, "a", &[]).unwrap();
    assert_eq!(p.flat.num_qubits, 4);
    assert_ne!(p.logical_to_physical[2], p.logical_to_physical[3]);
    let mut placed = p.flat;
    original.interface.extend([2, 3]);
    original.workspace.clear();
    placed.interface.extend([2, 3]);
    placed.workspace.clear();
    equivalent(&original, &placed);
}
#[test]
fn placed_fixture_actions_match_unplaced_exact_states() {
    let dir = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures";
    let mut actions = 0;
    let mut tuples = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|s| s != "bisa") {
            continue;
        }
        let o = Object::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        for f in o.functions.iter().filter(|f| f.is_action()) {
            let p = place_action(&o, &f.name, &[]).unwrap();
            let original = flatten(&lower_program(&o, &f.name).unwrap()).unwrap();
            assert!(p.flat.num_qubits <= original.num_qubits);
            tuples += equivalent(&original, &p.flat);
            actions += 1;
        }
    }
    assert_eq!((actions, tuples), (13, 740));
    eprintln!("PLACEMENT HARNESS: {actions} actions / {tuples} exact tuples agree");
}
#[test]
fn placement_rejects_unimplemented_schedule_consumption_by_upstream_bead() {
    let mut o = object("perm_action @a (t:wire mut) () () () ()\n^entry():\nx t\n");
    o.schedule = Some(Value::parse("{}").unwrap());
    let e = place_action(&o, "a", &[]).unwrap_err();
    assert!(
        e.0.contains("unimplemented:") && e.0.contains("b-isa-spec-8r4"),
        "{e}"
    );
}

#[test]
fn local_region_image_wires_are_pinned_and_validation_cannot_be_bypassed() {
    let path = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/actions.bisa";
    let text = std::fs::read_to_string(path).unwrap();
    let o = Object::parse(&text).unwrap();
    let p = place_action(&o, "memory", &[]).unwrap();
    let regions = p
        .lifetimes
        .iter()
        .filter(|l| l.kind == bennett_circuits::ir::WsKind::LocalRegion)
        .collect::<Vec<_>>();
    assert_eq!(regions.len(), 8);
    for r in regions {
        assert_eq!((r.start, r.end), (0, p.flat.gates.len()));
        assert!(
            p.lifetimes
                .iter()
                .all(|l| l.logical == r.logical || l.physical != r.physical)
        );
    }
    let invalid = Object::parse(&text.replace("quncopy p d", "x d[0]\nquncopy p d")).unwrap();
    let e = place_action(&invalid, "flip", &[]).unwrap_err();
    assert!(e.0.contains("§6.2"), "{e}");
}
