use bennett_circuits::{lower::lower_program, object::Object, qasm};

#[test]
fn fixture_roundtrips_and_executable_text_agrees() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bennett-isa-spec/reference-interpreter/tests/fixtures");
    let mut actions = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "bisa") {
            continue;
        }
        let obj = Object::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        for f in obj.functions.iter().filter(|f| f.is_action()) {
            let ir = lower_program(&obj, &f.name).unwrap();
            let text = qasm::export(&ir).unwrap();
            assert_eq!(qasm::parse(&text).unwrap(), ir);
            assert_eq!(
                qasm::parse_flat(&text).unwrap(),
                bennett_circuits::flatten::flatten(&ir).unwrap()
            );
            // Executable edits cannot be hidden by stale metadata.
            let bad = text.replacen("OPENQASM 3.0;", "OPENQASM 2.0;", 1);
            assert!(qasm::parse(&bad).is_err());
            actions += 1;
        }
    }
    assert_eq!(actions, 13);
}

#[test]
fn cli_exports_hierarchy_and_rejects_stale_metadata() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
        .args(["export-qasm", path.to_str().unwrap(), "controlled_phase"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("ctrl @ bc"));
    assert_eq!(qasm::parse(&text).unwrap().circuits.len(), 2);
    assert!(qasm::parse(&text.replace("ctrl @ bc", "negctrl @ bc")).is_err());
}

#[test]
fn cli_rejects_corrupted_stored_identity_before_export() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture =
        root.join("../bennett-isa-spec/reference-interpreter/tests/fixtures/control_flow.bisa");
    let text = std::fs::read_to_string(fixture).unwrap();
    let corrupt = text.replace("rz 6.283185307179586 t", "rz 3.0 t");
    assert_ne!(corrupt, text);
    std::fs::create_dir_all(root.join(".tmp")).unwrap();
    let path = root.join(".tmp/qasm-bad-identity.bisa");
    std::fs::write(&path, corrupt).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
        .args(["export-qasm", path.to_str().unwrap(), "phase"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("stored interface_hash")
    );
}
