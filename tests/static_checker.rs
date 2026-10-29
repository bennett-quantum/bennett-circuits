use bennett_circuits::{json::Value, object::Object};
use std::{path::Path, process::Command};
#[test]
fn suite_f_check_verdicts_and_rules() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let dir = workspace.join("bennett-isa-spec/conformance/cases/F");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path().join("case.json");
        let c = Value::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let Some(command) = c.get("command").and_then(Value::as_array) else {
            continue;
        };
        if command[0].as_str() != Some("check") {
            continue;
        }
        let obj = path
            .parent()
            .unwrap()
            .join(c.get("object").unwrap().as_str().unwrap());
        let text = std::fs::read_to_string(&obj).unwrap();
        let ours = Object::parse(&text).and_then(|o| o.check());
        let expected = c.get("expect").unwrap();
        let ok = expected.get("exit").unwrap().as_u64() == Some(0);
        assert_eq!(ours.is_ok(), ok, "{}: {ours:?}", path.display());
        if !ok {
            for rule in expected.get("stderr_contains").unwrap().as_array().unwrap() {
                assert!(
                    ours.as_ref()
                        .unwrap_err()
                        .0
                        .contains(rule.as_str().unwrap()),
                    "{}: {ours:?}",
                    path.display()
                );
            }
        }
        let output = Command::new(
            workspace.join("bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref"),
        )
        .args(["check", obj.to_str().unwrap()])
        .output()
        .unwrap();
        assert_eq!(output.status.success(), ours.is_ok());
        count += 1;
    }
    eprintln!("STATIC HARNESS: {count} suite F check verdicts compared");
}
#[test]
fn semantic_rejections_precede_stored_hashes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("bennett-isa-spec/conformance/cases/objects");
    for name in [
        "f_width_mismatch",
        "f_window_closed",
        "f_borrow_neg1",
        "f_borrow_neg2",
        "f_borrow_neg3",
        "f_borrow_neg4",
        "f_name_resolution",
        "f_oracle_gate",
        "f_no_terminator",
        "f_retval_in_lproc",
        "f_memoryless_load",
        "f_disjoint_pred",
        "f_in_wire_target",
        "f_action_cycle",
    ] {
        let path = dir.join(format!("{name}.bisa"));
        if !path.exists() {
            panic!("missing {}", path.display())
        }
        let text = std::fs::read_to_string(path).unwrap();
        assert!(
            Object::parse(&text).and_then(|o| o.hash()).is_err(),
            "{name}: hash must validate semantics, not just defer rejection to stored hashes"
        );
    }
}
#[test]
fn a_cell_version_advance_closes_load_cleanup_even_if_restored() {
    let path = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/actions.bisa";
    let text = std::fs::read_to_string(path).unwrap();
    let text = text.replace(
        "d = qload r p\nqunload r p d",
        "d = qload r p\nqstore r p t\nqstore r p t\nqunload r p d",
    );
    let error = Object::parse(&text).unwrap().hash().unwrap_err();
    assert!(error.0.contains("§6.2"), "{error}");
}

fn nested_object(body: &str) -> Object {
    let path = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/nested.bisa";
    let text = std::fs::read_to_string(path).unwrap();
    let old = "lift.oracle_call @inc8 tk (x) r fr () ()\nlift.oracle_uncall @inc8 tk (x) r fr () ()\nlift.retval r";
    Object::parse(&text.replace(old, body)).unwrap()
}
#[test]
fn matched_cleanup_crosses_cfg_edges_and_handles_loop_fixed_points() {
    for body in [
        "lift.oracle_call @inc8 tk (x) r fr () ()\nqjump ^cleanup\n^cleanup():\nlift.oracle_uncall @inc8 tk (x) r fr () ()\nlift.retval r",
        "qjump ^again\n^again():\nlift.oracle_call @inc8 tk (x) r fr () ()\nlift.oracle_uncall @inc8 tk (x) r fr () ()\nqbranch p ^again ^done\n^done():\nlift.retval r",
        "qbranch p ^left ^right\n^left():\nlift.oracle_call @inc8 tk (x) r fr () ()\nqjump ^cleanup\n^right():\nlift.oracle_call @inc8 tk (x) r fr () ()\nqjump ^cleanup\n^cleanup():\nlift.oracle_uncall @inc8 tk (x) r fr () ()\nlift.retval r",
    ] {
        nested_object(body).hash().unwrap();
    }
}
#[test]
fn matched_cleanup_requires_a_call_on_every_incoming_path() {
    let o = nested_object(
        "qbranch p ^left ^cleanup\n^left():\nlift.oracle_call @inc8 tk (x) r fr () ()\nqjump ^cleanup\n^cleanup():\nlift.oracle_uncall @inc8 tk (x) r fr () ()\nlift.retval r",
    );
    let e = o.hash().unwrap_err();
    assert!(
        e.0.contains("§5.5") && !e.0.contains("unimplemented"),
        "{e}"
    );
}
#[test]
fn matched_cleanup_rejects_input_write_through_another_call() {
    let mut o = nested_object(
        "lift.oracle_call @inc8 tk (r) zero fr () ()\nlift.oracle_call @inc8 tk (x) r p () ()\nlift.oracle_uncall @inc8 tk (r) zero fr () ()\nlift.retval r",
    );
    // Use well-typed output slots; all changes remain semantic hash tests.
    let f = o.functions.iter_mut().find(|f| f.name == "cancel").unwrap();
    f.regs.iter_mut().find(|p| p.name == "zero").unwrap().ty =
        bennett_circuits::object::Type::Reg(8);
    f.regs.iter_mut().find(|p| p.name == "p").unwrap().ty = bennett_circuits::object::Type::Reg(4);
    let e = o.hash().unwrap_err();
    assert!(e.0.contains("§5.5"), "{e}");
}

#[test]
fn all_conformance_objects_agree_in_verdict_and_rule_citation() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let dir = base.join("bennett-isa-spec/conformance/cases/objects");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_none_or(|x| x != "bisa") {
            continue;
        }
        let ours = Object::parse(&std::fs::read_to_string(&p).unwrap()).and_then(|o| o.check());
        let reference = Command::new(
            base.join("bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref"),
        )
        .arg("check")
        .arg(&p)
        .output()
        .unwrap();
        assert_eq!(
            ours.is_ok(),
            reference.status.success(),
            "{}: ours={ours:?}; ref={}",
            p.display(),
            String::from_utf8_lossy(&reference.stderr)
        );
        if let Err(e) = ours {
            let diagnostic = String::from_utf8(reference.stderr).unwrap();
            let rule = diagnostic
                .split_once('§')
                .expect("reference rule citation")
                .1
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .next()
                .unwrap()
                .trim_end_matches('.');
            assert!(
                e.0.contains(&format!("§{rule}")),
                "{}: ours={e}; ref={diagnostic}",
                p.display()
            );
        }
        count += 1;
    }
    assert_eq!(count, 42);
    eprintln!("STATIC HARNESS: {count} conformance objects agree in verdict and rule citation");
}
