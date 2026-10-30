#[path = "support/exact.rs"]
#[allow(dead_code)]
mod exact;
#[path = "support/sim.rs"]
mod sim;
use bennett_circuits::{
    object::Object,
    placement::place_on_target,
    target::{Rational, Target},
};
const ROOT: &str = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/E";
#[test]
fn published_exact_target_load_and_forbidden_eps_reference() {
    let text = std::fs::read_to_string(format!("{ROOT}/E-epsilon-zero-exact/target.json")).unwrap();
    let t = Target::parse(&text).unwrap();
    assert_eq!(t.hash.len(), 64);
    let bad = std::fs::read_to_string(format!("{ROOT}/E-epsilon-zero-entry/target.json")).unwrap();
    assert!(Target::parse(&bad).unwrap_err().0.contains("§8.1"));
    assert!(
        Target::parse(&(text + "\n"))
            .unwrap_err()
            .0
            .contains("canonical")
    );
}
#[test]
fn target_placement_matches_unplaced_fixture_states() {
    let target = r#"{"capabilities":{"classical_step_time":1,"max_parallel":"unbounded","max_predicate_depth":"unbounded","mbu":false,"measurement":{"feedforward_latency":0,"mid_circuit":false}},"classes":{"fast":{"capacity":1000,"dirty_rate":1,"qubit_second":1}},"gates":[{"classes":["*"],"controls":{"max":"unbounded","min":0},"cost":{"gates":1,"time":1},"epsilon":0,"op":"x"}],"imports":{},"objective":{"weighted":{"time":1}},"ptr_widths_supported":[32],"qram":null,"resources":{"gates":{"kind":"count","unit":"gate"},"peak_qubits":{"kind":"peak","unit":"qubit"},"qubit_second":{"kind":"occupancy","unit":"qubit_second"},"time":{"kind":"time","unit":"second"}},"schemes":[],"tables":{}}"#;
    let mut value = bennett_circuits::json::Value::parse(target).unwrap();
    let gates = if let bennett_circuits::json::Value::Object(m) = &mut value {
        m.get_mut("gates").unwrap()
    } else {
        unreachable!()
    };
    let entry = gates.as_array().unwrap()[0].clone();
    *gates = bennett_circuits::json::Value::Array(
        [
            "x", "cx", "ccx", "mcx", "h", "s", "sdg", "t", "tdg", "cz", "swap", "rx", "ry", "rz",
        ]
        .iter()
        .map(|op| {
            let mut g = entry.clone();
            if let bennett_circuits::json::Value::Object(m) = &mut g {
                m.insert(
                    "op".into(),
                    bennett_circuits::json::Value::Str((*op).into()),
                );
                if ["cz", "swap"].contains(op) {
                    m.insert(
                        "classes".into(),
                        bennett_circuits::json::Value::parse(r#"["*","*"]"#).unwrap(),
                    );
                }
                if ["rx", "ry", "rz"].contains(op) {
                    m.insert(
                        "params".into(),
                        bennett_circuits::json::Value::parse(r#"["theta"]"#).unwrap(),
                    );
                }
            }
            g
        })
        .collect(),
    );
    let t = Target::parse(&value.to_jcs()).unwrap();
    let mut count = 0;
    for e in std::fs::read_dir("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures").unwrap() {
        let path=e.unwrap().path(); if path.extension().is_none_or(|s|s!="bisa") {continue}
        let o=Object::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        for f in o.functions.iter().filter(|f|f.is_action()) {
            let p=place_on_target(&o,&f.name,&[],&t).unwrap();
            let old=bennett_circuits::flatten::flatten(&bennett_circuits::lower::lower_program(&o,&f.name).unwrap()).unwrap();
            for x in 0..1usize<<old.interface.len() {
                let bits=(0..old.interface.len()).map(|i|if x>>i&1==0{'0'}else{'1'}).collect::<String>();
                assert_eq!(sim::basis(&old,&bits),sim::basis(&p.flat,&bits),"{} {bits}",f.name);count+=1;
            }
            assert_eq!(p.resources["gates"], Rational::integer(p.events.len() as i128));
            assert!(p.classes.iter().all(|c|c=="fast"));
            let q=bennett_circuits::qasm::export_placed(&p).unwrap();
            let parsed=bennett_circuits::qasm::parse_flat(&q).unwrap();
            assert_eq!(p.flat,parsed);
        }
    }
    assert_eq!(count, 740);
    eprintln!("TARGET PLACEMENT: {count} exact placed/unplaced tuples agree");
}

fn val(s: &str) -> bennett_circuits::json::Value {
    bennett_circuits::json::Value::parse_target(s).unwrap()
}
fn base_target() -> bennett_circuits::json::Value {
    val(
        r#"{"capabilities":{"classical_step_time":1,"max_parallel":"unbounded","max_predicate_depth":"unbounded","mbu":false,"measurement":{"feedforward_latency":0,"mid_circuit":false}},"classes":{"fast":{"capacity":100,"dirty_rate":0.25,"qubit_second":7}},"gates":[{"classes":["*"],"controls":{"min":0,"max":"unbounded"},"cost":{"time":1,"gates":1},"epsilon":0,"op":"x"},{"classes":["*"],"controls":{"min":1,"max":"unbounded"},"cost":{"time":1,"gates":1},"epsilon":0,"op":"cx"},{"classes":["*"],"controls":{"min":0,"max":"unbounded"},"cost":{"time":1,"gates":1},"epsilon":0,"op":"h"}],"imports":{},"objective":{"weighted":{"time":1}},"ptr_widths_supported":[32],"qram":null,"resources":{"gates":{"kind":"count","unit":"gate"},"peak_qubits":{"kind":"peak","unit":"qubit"},"qubit_second":{"kind":"occupancy","unit":"qs"},"time":{"kind":"time","unit":"s"}},"schemes":[],"tables":{}}"#,
    )
}
fn put(v: &mut bennett_circuits::json::Value, k: &str, x: bennett_circuits::json::Value) {
    if let bennett_circuits::json::Value::Object(m) = v {
        m.insert(k.into(), x);
    } else {
        panic!("not object")
    }
}
fn make_object(body: &str) -> Object {
    use bennett_circuits::object::{Symbol, derived_signature};
    let mut o=Object::parse(&format!("bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\n{body}")).unwrap();
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
fn scheme_target() -> bennett_circuits::json::Value {
    let mut t = base_target();
    let mut gs = t.get("gates").unwrap().as_array().unwrap().to_vec();
    put(&mut gs[0], "controls", val(r#"{"min":0,"max":0}"#));
    put(&mut t, "gates", bennett_circuits::json::Value::Array(gs));
    put(
        &mut t,
        "schemes",
        val(
            r#"[{"name":"controlled_x","realises":{"op":"x","controls":{"min":1,"max":1}},"ancillae":{"count":{"var":"c"},"class":"fast","state":"clean"},"template":[{"gate":"cx","wires":["c[0]","a[0]"]},{"gate":"cx","wires":["a[0]","t"]},{"gate":"cx","wires":["c[0]","a[0]"]}]}]"#,
        ),
    );
    t
}
#[test]
fn scheme_expansion_is_exact_accounts_leaves_and_reuses_ancilla_frames() {
    let t = Target::parse(&scheme_target().to_jcs()).unwrap();
    let o = make_object(
        "perm_action @sub (t:wire mut) () () () ()\n^entry():\nx t\naction @a (t:wire mut,p:wire mut) () () () ()\n^entry():\nh p\nctl p action_call @sub (t) () () ()\nctl p adj action_call @sub (t) () () ()\nh p\n",
    );
    let p = place_on_target(&o, "a", &[], &t).unwrap();
    assert_eq!((p.flat.num_qubits, p.events.len()), (3, 8));
    assert_eq!(p.resources["gates"], Rational::integer(8));
    assert_eq!(p.resources["time"], Rational::integer(8));
    assert_eq!(p.per_class["peak_qubits"]["fast"], Rational::integer(3));
    assert_eq!(p.per_class["qubit_second"]["fast"], Rational::integer(22));
    let old = bennett_circuits::flatten::flatten(
        &bennett_circuits::lower::lower_program(&o, "a").unwrap(),
    )
    .unwrap();
    for bits in ["00", "01", "10", "11"] {
        assert_eq!(sim::basis(&old, bits), sim::basis(&p.flat, bits));
        let z = sim::basis(&p.flat, bits);
        assert_eq!(z[bits].value(), (1., 0.));
    }
    assert_eq!(
        p.events.iter().filter(|e| e.leaf.scheme.is_some()).count(),
        6
    );
    let mut low = scheme_target();
    put(
        &mut low,
        "classes",
        val(r#"{"fast":{"capacity":2,"dirty_rate":1,"qubit_second":1}}"#),
    );
    assert!(
        place_on_target(&o, "a", &[], &Target::parse(&low.to_jcs()).unwrap())
            .unwrap_err()
            .0
            .contains("§8.4")
    );
}
#[test]
fn exact_expressions_binding_epsilon_and_first_entry() {
    use bennett_circuits::target::evaluate;
    use std::collections::BTreeMap;
    let vars = BTreeMap::from([("eps".into(), Rational::parse("0.001").unwrap())]);
    assert_eq!(
        evaluate(
            &val(r#"{"op":"log2inv_eps","args":[]}"#),
            &vars,
            &BTreeMap::new()
        )
        .unwrap(),
        Rational::integer(10)
    );
    assert!(
        evaluate(&val(r#"{"op":"/","args":[1,0]}"#), &vars, &BTreeMap::new())
            .unwrap_err()
            .0
            .contains("division by zero")
    );
    assert_eq!(
        Rational::parse("1e-3")
            .unwrap()
            .mul(Rational::integer(1000))
            .unwrap(),
        Rational::ONE
    );
    let mut t = base_target();
    let mut entries = t.get("gates").unwrap().as_array().unwrap().to_vec();
    let mut approx = entries[0].clone();
    put(&mut approx, "epsilon", val("0.01"));
    put(&mut approx, "cost", val(r#"{"time":9,"gates":9}"#));
    entries.insert(0, approx);
    put(
        &mut t,
        "gates",
        bennett_circuits::json::Value::Array(entries),
    );
    let t = Target::parse(&t.to_jcs()).unwrap();
    let gate = bennett_circuits::flatten::FlatGate {
        gate: bennett_circuits::ir::BaseGate::X,
        controls: vec![],
        targets: vec![0],
    };
    let leaf = t
        .native(
            "x",
            &gate,
            0,
            &["fast".into()],
            Rational::parse("0.001").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(leaf.costs["time"], Rational::ONE);
    let leaf = t
        .native(
            "x",
            &gate,
            0,
            &["fast".into()],
            Rational::parse("0.1").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(leaf.costs["time"], Rational::integer(9));
    let mut bad = base_target();
    let mut gs = bad.get("gates").unwrap().as_array().unwrap().to_vec();
    put(&mut gs[0], "cost", val(r#"{"time":{"var":"theta"}}"#));
    put(&mut bad, "gates", bennett_circuits::json::Value::Array(gs));
    assert!(
        Target::parse(&bad.to_jcs())
            .unwrap_err()
            .0
            .contains("unbound")
    );
}
#[test]
fn any_class_capacity_and_own_versus_effective_controls() {
    let mut t = base_target();
    put(
        &mut t,
        "classes",
        val(
            r#"{"fast":{"capacity":1,"dirty_rate":1,"qubit_second":1},"slow":{"capacity":1,"dirty_rate":1,"qubit_second":1}}"#,
        ),
    );
    let mut gs = t.get("gates").unwrap().as_array().unwrap().to_vec();
    put(
        &mut gs[0],
        "cost",
        val(r#"{"time":{"op":"+","args":[1,{"var":"c"}]},"gates":{"var":"k"}}"#),
    );
    put(&mut t, "gates", bennett_circuits::json::Value::Array(gs));
    let o = make_object(
        "perm_action @sub (t:wire mut) () () () ()\n^entry():\nx t\nperm_action @a (t:wire mut) (p:wire in) () () ()\n^entry():\nctl p action_call @sub (t) () () ()\n",
    );
    let p = place_on_target(&o, "a", &[], &Target::parse(&t.to_jcs()).unwrap()).unwrap();
    assert_eq!(p.classes, vec!["fast", "slow"]);
    assert_eq!(p.resources["gates"], Rational::ZERO);
    assert_eq!(p.resources["time"], Rational::integer(2));
    let mut fail = t;
    put(
        &mut fail,
        "classes",
        val(r#"{"fast":{"capacity":1,"dirty_rate":1,"qubit_second":1}}"#),
    );
    assert!(
        place_on_target(&o, "a", &[], &Target::parse(&fail.to_jcs()).unwrap())
            .unwrap_err()
            .0
            .contains("§8.4")
    );
}
#[test]
fn topology_schedule_and_missing_native_leaf_fail_loudly() {
    let mut v = base_target();
    put(&mut v, "topology", val("{}"));
    assert!(
        Target::parse(&v.to_jcs())
            .unwrap_err()
            .0
            .contains("bennett-circuits-b4k")
    );
    let mut v = scheme_target();
    let gs = v
        .get("gates")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g.get("op").unwrap().as_str() != Some("cx"))
        .cloned()
        .collect();
    put(&mut v, "gates", bennett_circuits::json::Value::Array(gs));
    let mut o = make_object(
        "perm_action @a (t:wire mut) (p:wire in) () () ()\n^entry():\nqpred p\nx t\nqunpred p\n",
    );
    let t = Target::parse(&v.to_jcs()).unwrap();
    assert!(
        place_on_target(&o, "a", &[], &t)
            .unwrap_err()
            .0
            .contains("schemes do not nest")
    );
    o.schedule = Some(val("{}"));
    assert!(
        place_on_target(&o, "a", &[], &t)
            .unwrap_err()
            .0
            .contains("b-isa-spec-8r4")
    );
}
