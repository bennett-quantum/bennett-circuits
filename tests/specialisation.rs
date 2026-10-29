#[path = "support/exact.rs"]
mod exact;
#[path = "support/sim.rs"]
mod sim;
use bennett_circuits::{
    flatten::flatten,
    lower::lower_program_with_args,
    object::Object,
    oracle::{Argument, Budget},
    qasm,
};
#[test]
fn qinplace_all_basis_and_adjoint() {
    let o=Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects/qinplace.bisa").unwrap()).unwrap();
    let p = lower_program_with_args(
        &o,
        "inc_inplace",
        &[Argument::Token(Budget {
            steps: 10,
            heap: 0,
            stack: 0,
            depth: 0,
        })],
    )
    .unwrap();
    let text = qasm::export(&p).unwrap();
    assert_eq!(qasm::parse(&text).unwrap(), p);
    let flat = qasm::parse_flat(&text).unwrap();
    for x in 0..4 {
        let bits = format!("{}{}", x & 1, (x >> 1) & 1);
        let y = (x + 1) % 4;
        let key = format!("{}{}", y & 1, (y >> 1) & 1);
        assert_eq!(sim::basis(&flat, &bits)[&key].value(), (1.0, 0.0));
    }
    let mut inverse = p.clone();
    inverse.circuits[0] = inverse.circuits[0].adjoint();
    let f = flatten(&inverse).unwrap();
    assert!(sim::basis(&f, "00").contains_key("11"));
}
fn parsed(body: &str) -> Object {
    Object::parse(&format!("bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\n{body}")).unwrap()
}
#[test]
fn classical_counts_angles_and_call_arguments_specialise() {
    let o = parsed(
        "action @leaf (t:wire mut) () () (theta:f64, n:i8) ()\nregs {  } bounds {3}\n^entry():\nqloop n {\nrx theta t\n}\naction @parent (t:wire mut) () () (theta:f64, n:i8) ()\n^entry():\naction_call @leaf (t) () () (theta, n)\nadj action_call @leaf (t) () () (theta, n)\n",
    );
    let p = lower_program_with_args(
        &o,
        "parent",
        &[Argument::Float(0.125), Argument::Integer("3".into())],
    )
    .unwrap();
    assert_eq!(p.circuits.len(), 2);
    assert_eq!(p.circuits[1].body.len(), 3);
    let flat = flatten(&p).unwrap();
    for bits in ["0", "1"] {
        assert_eq!(sim::basis(&flat, bits)[bits].value(), (1.0, 0.0));
    }
}
#[test]
fn controlled_adjoint_oracle_calls_include_cleanup_gates() {
    let o = parsed(
        "oracle @inc (x:wire[2] in) (t:wire[2]) (f:wire[4]) () ()\nregs { one:reg[2], r:reg[2] }\n^entry():\none = lift.const.i2 1\nr = lift.add x one\nlift.retval r\nperm_action @controlled (t:wire[2] mut, f:wire[4] mut) (x:wire[2] in, p:wire in) () (tk:token) ()\n^entry():\nctl p adj oracle_call @inc tk (x) t f () ()\n",
    );
    let p = lower_program_with_args(
        &o,
        "controlled",
        &[Argument::Token(Budget {
            steps: 3,
            heap: 0,
            stack: 0,
            depth: 0,
        })],
    )
    .unwrap();
    let text = qasm::export(&p).unwrap();
    let flat = qasm::parse_flat(&text).unwrap();
    for column in 0..512usize {
        let bits = (0..9)
            .map(|i| if column >> i & 1 == 1 { '1' } else { '0' })
            .collect::<String>();
        let x = (column >> 6) & 3;
        let value = if column >> 8 == 1 {
            column ^ ((x + 1) & 3)
        } else {
            column
        };
        let key = (0..9)
            .map(|i| if value >> i & 1 == 1 { '1' } else { '0' })
            .collect::<String>();
        assert_eq!(sim::basis(&flat, &bits)[&key].value(), (1.0, 0.0));
    }
}
#[test]
fn integer_specialisation_arguments_are_range_checked() {
    let o = parsed("action @a (t:wire mut) () () (n:i8) ()\n^entry():\nx t\n");
    for n in ["256", "-129", "garbage"] {
        assert!(
            lower_program_with_args(&o, "a", &[Argument::Integer(n.into())]).is_err(),
            "{n}"
        );
    }
    for n in ["255", "-128"] {
        assert!(
            lower_program_with_args(&o, "a", &[Argument::Integer(n.into())]).is_ok(),
            "{n}"
        );
    }
}
#[test]
fn cli_exports_a_budget_specialised_oracle() {
    let output=std::process::Command::new(env!("CARGO_BIN_EXE_bennett-circuits")).args(["export-qasm","/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/inc8.bisa","inc8","--budget","0,0,0,0"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let flat = qasm::parse_flat(&text).unwrap();
    let out = sim::basis(&flat, "00000000000000000000");
    assert_eq!(out["00000000100000000000"].value(), (1.0, 0.0));
}
#[test]
fn qinplace_requires_fault_free_inverse_pair_under_the_actual_token() {
    let text=std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects/qinplace.bisa").unwrap();
    for (text, steps) in [
        (text.clone(), 0),
        (
            text.replace("qinplace @inc2 @dec2", "qinplace @inc2 @inc2"),
            10,
        ),
    ] {
        let o = Object::parse(&text).unwrap();
        let error = lower_program_with_args(
            &o,
            "inc_inplace",
            &[Argument::Token(Budget {
                steps,
                heap: 0,
                stack: 0,
                depth: 0,
            })],
        )
        .unwrap_err();
        assert!(error.0.contains("§4.5"), "{error}");
    }
}
#[test]
fn compiled_basis_cli_serves_suite_f_execution_cases() {
    use bennett_circuits::json::Value;
    let base = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects";
    for (args, field, value) in [
        (
            vec![
                "run-oracle".to_string(),
                format!("{base}/mutual.bisa"),
                "main".into(),
                "--input".into(),
                "".into(),
                "--budget".into(),
                "20,0,32,4".into(),
            ],
            "payload",
            4,
        ),
        (
            vec![
                "run-action".into(),
                format!("{base}/qinplace.bisa"),
                "inc_inplace".into(),
                "--mut".into(),
                "0".into(),
                "--arg".into(),
                "token:10,0,0,0".into(),
                "--adjoint".into(),
            ],
            "mut",
            3,
        ),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let result = Value::parse(&String::from_utf8(out.stdout).unwrap()).unwrap();
        if field == "mut" {
            assert_eq!(
                result.get(field).unwrap().as_array().unwrap()[0].as_u64(),
                Some(value)
            );
        } else {
            assert_eq!(result.get(field).unwrap().as_u64(), Some(value));
            assert_eq!(result.get("steps").unwrap().as_u64(), Some(6));
        }
    }
}
#[test]
fn call_boundary_disjointness_applies_even_to_zero_trip_callees() {
    let leaf = "perm_action @nop (a:wire mut, b:wire mut) () () () ()\nregs {  } bounds {0}\n^entry():\nqloop 0 {\nx a\n}\n";
    for body in [
        "action_call @nop (t, t) () () ()\n",
        "qpred t\naction_call @nop (t, p) () () ()\nqunpred t\n",
    ] {
        let o = parsed(&format!(
            "{leaf}perm_action @parent (t:wire mut, p:wire mut) () () () ()\n^entry():\n{body}"
        ));
        let error = lower_program_with_args(&o, "parent", &[]).unwrap_err();
        assert!(error.0.contains("§3.3"), "{error}");
    }
}
#[test]
fn pointer_specialisation_rejects_invalid_or_out_of_range_values() {
    let o = parsed("action @a (t:wire mut) () () (p:ptr) ()\n^entry():\nx t\n");
    for n in ["garbage", "4294967296", "-2147483649"] {
        assert!(
            lower_program_with_args(&o, "a", &[Argument::Integer(n.into())]).is_err(),
            "{n}"
        );
    }
    for n in ["4294967295", "-2147483648"] {
        assert!(
            lower_program_with_args(&o, "a", &[Argument::Integer(n.into())]).is_ok(),
            "{n}"
        );
    }
}

#[test]
fn basis_diagnostics_do_not_materialise_halt_padding_for_large_budgets() {
    let o=Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/inc8.bisa").unwrap()).unwrap();
    let (payload, code, steps) = bennett_circuits::proof::oracle_basis(
        &o,
        "inc8",
        Budget {
            steps: 1_000_000,
            heap: 0,
            stack: 0,
            depth: 0,
        },
        &[],
        &[vec![false; 8]],
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        payload,
        vec![true, false, false, false, false, false, false, false]
    );
    assert_eq!(code, vec![false; 4]);
    assert_eq!(steps, 3);
}
