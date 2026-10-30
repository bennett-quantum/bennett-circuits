//! Published acceptance probes, read only; no implementation imports.
use bennett_circuits::json::Value;
use std::{path::Path, process::Command};
fn contains(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => e
            .iter()
            .all(|(k, v)| a.get(k).is_some_and(|a| contains(a, v))),
        (Value::Array(a), Value::Array(e)) => {
            a.len() == e.len() && a.iter().zip(e).all(|(a, e)| contains(a, e))
        }
        _ => actual == expected,
    }
}
#[test]
fn suite_a_execution_and_reversal() {
    acceptance("A", |c| {
        if c.get("command").is_none() {
            assert!(c.get("unavailable").is_some(), "unclassified case: {c}");
            eprintln!(
                "A unavailable (published staged case): {}",
                c.get("id").unwrap()
            );
            false
        } else {
            true
        }
    });
}
#[test]
fn suite_c_action_reversal_and_subbyte_memory() {
    acceptance("C", |c| {
        matches!(
            c.get("id").and_then(Value::as_str),
            Some("C-action-reverse-adjoint" | "C-qmem-exchange")
        )
    });
}
#[test]
fn suite_h_expansion_refusal() {
    acceptance("H", |c| c.get("command").is_some());
}
#[test]
fn suite_d_exact_actions() {
    acceptance("D", |c| {
        c.get("command").is_some() && c.get("unavailable").is_none()
    });
}
#[test]
fn suite_g_one_shot_launch() {
    acceptance("G", |c| {
        c.get("command").is_some() && c.get("unavailable").is_none()
    });
}
#[test]
fn suite_b_prefix_reversal_and_step_check() {
    acceptance("B", |c| {
        matches!(
            c.get("command").unwrap().as_array().unwrap()[0].as_str(),
            Some("reverse" | "step-check")
        )
    });
}
fn acceptance(suite: &str, include: impl Fn(&Value) -> bool) {
    let cases = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(format!("bennett-isa-spec/conformance/cases/{suite}"));
    for entry in std::fs::read_dir(cases).unwrap() {
        let dir = entry.unwrap().path();
        let c =
            Value::parse_target(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        if !include(&c) {
            continue;
        }
        let object = dir.join(c.get("object").unwrap().as_str().unwrap());
        let args = c
            .get("command")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                v.as_str()
                    .unwrap()
                    .replace("{object}", object.to_str().unwrap())
                    .replace("{dir}", dir.to_str().unwrap())
            })
            .collect::<Vec<_>>();
        let output = Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
            .args(args)
            .output()
            .unwrap();
        let expect = c.get("expect").unwrap();
        assert_eq!(
            output.status.code().unwrap() as u64,
            expect.get("exit").unwrap().as_u64().unwrap(),
            "{}: {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(json) = expect.get("stdout_json") {
            let actual = Value::parse_target(&String::from_utf8(output.stdout).unwrap()).unwrap();
            assert!(
                contains(&actual, json),
                "{}: actual={actual:?}; expected={json:?}",
                dir.display()
            );
        }
        if let Some(ss) = expect.get("stderr_contains") {
            for s in ss.as_array().unwrap() {
                assert!(String::from_utf8_lossy(&output.stderr).contains(s.as_str().unwrap()));
            }
        }
    }
}
