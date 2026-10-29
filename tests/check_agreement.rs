//! `check` agreement with `bennett-isa-ref check` (brief milestone 1,
//! deliverable 2): same verdict, same `object_hash`, same per-symbol
//! interface/closure hashes, on every spec example (§16.3) and every
//! fixture. The reference binary is used through its CLI only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use bennett_circuits::json::Value;
use bennett_circuits::object::Object;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace parent")
        .to_path_buf()
}

fn ref_bin() -> PathBuf {
    let p = workspace().join("bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref");
    assert!(
        p.exists(),
        "differential oracle missing: {} (build bennett-isa-spec first)",
        p.display()
    );
    p
}

fn fixtures_dir() -> PathBuf {
    workspace().join("bennett-isa-spec/reference-interpreter/tests/fixtures")
}

fn ref_check(path: &Path) -> Result<Value, String> {
    let out = Command::new(ref_bin())
        .arg("check")
        .arg(path)
        .output()
        .expect("run bennett-isa-ref check");
    if out.status.success() {
        let s = String::from_utf8(out.stdout).expect("ref stdout utf8");
        Value::parse(s.trim()).map_err(|e| format!("ref JSON: {e}"))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

fn json_str<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string field {key} in {v}"))
}

fn assert_agrees(label: &str, text: &str, path: &Path) {
    let reference = ref_check(path).unwrap_or_else(|e| panic!("{label}: ref check failed: {e}"));
    let obj = Object::parse(text).unwrap_or_else(|e| panic!("{label}: our parse failed: {e}"));
    let report = obj
        .check()
        .unwrap_or_else(|e| panic!("{label}: our check failed: {e}"));

    assert_eq!(
        report.object_hash,
        json_str(&reference, "object_hash"),
        "{label}: object_hash"
    );
    assert_eq!(
        report.status,
        json_str(&reference, "status"),
        "{label}: status"
    );
    let ref_symbols = reference
        .get("symbols")
        .and_then(Value::as_object)
        .expect("ref symbols object");
    let mut ref_map: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    for (name, entry) in ref_symbols {
        let ih = json_str(entry, "interface_hash").to_string();
        let ch = entry
            .get("closure_hash")
            .and_then(Value::as_str)
            .map(str::to_string);
        ref_map.insert(name.clone(), (ih, ch));
    }
    let our_map: BTreeMap<String, (String, Option<String>)> = report
        .symbols
        .iter()
        .map(|(n, h)| {
            (
                n.clone(),
                (h.interface_hash.clone(), h.closure_hash.clone()),
            )
        })
        .collect();
    assert_eq!(our_map, ref_map, "{label}: symbol hashes");
}

#[test]
fn check_agrees_on_every_fixture() {
    let dir = fixtures_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read fixtures dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "bisa"))
        .collect();
    paths.sort();
    assert!(
        paths.len() >= 12,
        "expected at least 12 fixtures, found {}",
        paths.len()
    );
    for path in &paths {
        let text = std::fs::read_to_string(path).expect("read fixture");
        assert_agrees(&path.display().to_string(), &text, path);
    }
}

/// Extract the §16.3 example objects (fenced blocks starting with
/// `bennett-object`) from the published spec and check agreement on them.
#[test]
fn check_agrees_on_spec_examples() {
    let spec =
        std::fs::read_to_string(workspace().join("bennett-isa-spec/spec/Bennett-ISA-Spec.md"))
            .expect("read ISA spec");
    let mut examples: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in spec.lines() {
        if line == "```" {
            if let Some(done) = current.take() {
                examples.push(done);
            } else {
                current = Some(String::new());
            }
        } else if let Some(buf) = current.as_mut() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    let objects: Vec<&String> = examples
        .iter()
        .filter(|b| b.starts_with("bennett-object"))
        .collect();
    assert_eq!(objects.len(), 2, "expected 2 example objects in §16.3");

    let tmp = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/spec-examples");
    std::fs::create_dir_all(&tmp).expect("create .tmp/spec-examples");
    for (i, text) in objects.iter().enumerate() {
        let path = tmp.join(format!("spec-example-{i}.bisa"));
        std::fs::write(&path, text).expect("write spec example");
        assert_agrees(&format!("spec example {i}"), text, &path);
    }
}

#[test]
fn inc8_envelope_matches_spec_bytes() {
    let text = std::fs::read_to_string(fixtures_dir().join("inc8.bisa")).expect("read inc8");
    let obj = Object::parse(&text).expect("parse inc8");
    let envelope = obj.semantic_envelope("inc8").expect("envelope");
    // The exact bytes printed in §16.3 example 1.
    let expected = concat!(
        "isa=0.8\n",
        "ptr_width=32\n",
        "{\"abi\":\"0.8\",\"args\":[],\"fault\":4,\"in\":[8],\"kind\":\"oracle\",\"mem\":[],\"tgt\":8,\"tier\":\"q\"}\n",
        "oracle @inc8 (x:wire[8] in) (t:wire[8]) (f:wire[4]) () ()\n",
        "regs { one:reg[8], r:reg[8] }\n",
        "^entry():\n",
        "one = lift.const.i8 1\n",
        "r = lift.add x one\n",
        "lift.retval r\n",
    );
    assert_eq!(String::from_utf8(envelope).expect("utf8"), expected);
}

#[test]
fn corrupted_objects_rejected_by_both() {
    let text = std::fs::read_to_string(fixtures_dir().join("inc8.bisa")).expect("read inc8");
    let tmp = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/spec-examples");
    std::fs::create_dir_all(&tmp).expect("create tmp");

    for (label, mutated) in [
        (
            "stored hash corrupted",
            text.replacen("8ce5e64f", "00000000", 1),
        ),
        (
            "body corrupted",
            text.replacen("lift.const.i8 1", "lift.const.i8 2", 1),
        ),
        (
            "signature corrupted",
            text.replacen("\"tgt\":8", "\"tgt\":9", 1),
        ),
    ] {
        assert_ne!(mutated, text, "{label}: mutation did not apply");
        let path = tmp.join("corrupt.bisa");
        std::fs::write(&path, &mutated).expect("write corrupt");
        assert!(
            ref_check(&path).is_err(),
            "{label}: ref accepted corruption"
        );
        let ours = Object::parse(&mutated).and_then(|o| o.check());
        assert!(ours.is_err(), "{label}: we accepted corruption: {ours:?}");
    }
}
