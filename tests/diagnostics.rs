//! Published acceptance probes, read only; no implementation imports.
#[path = "support/corpus.rs"]
mod corpus;
use bennett_circuits::json::Value;
use corpus::Declared;
use std::{path::Path, process::Command};
#[test]
fn corpus_selection_rule_is_by_declared_isa_version() {
    let implemented = corpus::implemented_isa();
    assert_eq!(implemented, "0.8", "SPEC_VERSIONS.toml isa pin");
    let header = |isa: &str, rest: &str| {
        format!(
            "bennett-object isa={isa} object={rest} launch={rest} sched={rest} cert={rest} coverage={rest} target={rest} ptr_width=32\nsection symbols\n"
        )
    };
    let case =
        |spec: &str| Value::parse(&format!("{{\"id\":\"X-1\",\"spec\":\"{spec}\"}}")).unwrap();
    let plain = case("§9.1 schemas");
    // A header declaring the implemented isa selects the case for its full expectations,
    // even when another version field differs (that is a 0.8 rejection case).
    assert_eq!(
        corpus::declared(&plain, Some(&header("0.8", "0.8")), &implemented),
        Declared::Implemented
    );
    assert_eq!(
        corpus::declared(&plain, Some(&header("0.8", "0.7")), &implemented),
        Declared::Implemented
    );
    // Any other declared isa is foreign: older (the 0.8 version-rejection case) or newer.
    for v in ["0.7", "0.9"] {
        assert_eq!(
            corpus::declared(&plain, Some(&header(v, v)), &implemented),
            Declared::Foreign(v.to_string())
        );
    }
    assert_eq!(
        corpus::declared(
            &case("ISA0.9 §12.3 K84"),
            Some(&header("0.9", "0.9")),
            &implemented
        ),
        Declared::Foreign("0.9".into())
    );
    // Nothing declares a version: no object, or a first line that is not a header.
    assert_eq!(
        corpus::declared(&plain, None, &implemented),
        Declared::Unversioned
    );
    assert_eq!(
        corpus::declared(&plain, Some("garbage isa=0.9\n"), &implemented),
        Declared::Unversioned
    );
    // The corpus' own citation label must agree with the object header; a mismatch is loud.
    let mislabelled = std::panic::catch_unwind(|| {
        corpus::declared(&case("ISA0.9 §11 K83"), Some(&header("0.8", "0.8")), "0.8")
    });
    assert!(
        mislabelled.is_err(),
        "ISA0.9-labelled case with an isa=0.8 object must panic"
    );
    // Tally: per-suite counts are printed and a suite that runs nothing at the implemented
    // version is a failure, not a pass.
    let mut t = corpus::Tally::new("X");
    t.record("X-a", &Declared::Implemented);
    t.record("X-b", &Declared::Unversioned);
    t.record("X-c", &Declared::Foreign("0.9".into()));
    assert_eq!(t.finish(), (1, 1, 1));
    let mut vacuous = corpus::Tally::new("Y");
    vacuous.record("Y-a", &Declared::Foreign("0.9".into()));
    assert!(std::panic::catch_unwind(move || vacuous.finish()).is_err());
}
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
    let implemented = corpus::implemented_isa();
    let mut tally = corpus::Tally::new(suite);
    for entry in std::fs::read_dir(cases).unwrap() {
        let dir = entry.unwrap().path();
        let c =
            Value::parse_target(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        if !include(&c) {
            continue;
        }
        let object = dir.join(c.get("object").unwrap().as_str().unwrap());
        let object_text = std::fs::read_to_string(&object)
            .unwrap_or_else(|e| panic!("{}: {e}", object.display()));
        let declared = corpus::declared(&c, Some(&object_text), &implemented);
        tally.record(c.get("id").unwrap().as_str().unwrap(), &declared);
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
        if let Declared::Foreign(v) = &declared {
            corpus::assert_refused(
                &dir.display().to_string(),
                v,
                output.status.code(),
                &String::from_utf8_lossy(&output.stdout),
                &String::from_utf8_lossy(&output.stderr),
            );
            continue;
        }
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
    tally.finish();
}
