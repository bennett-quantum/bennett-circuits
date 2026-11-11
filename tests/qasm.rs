use super::fixtures;
use super::sibling;
use bennett_circuits::{object::Object, qasm};

#[test]
fn fixture_roundtrips_and_executable_text_agrees() {
    let dir = sibling::isa().join("reference-interpreter/tests/fixtures");
    let mut actions = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "bisa") {
            continue;
        }
        let obj = Object::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        for f in obj.functions.iter().filter(|f| f.is_action()) {
            let ir = bennett_circuits::lower::lower_program_with_args(
                &obj,
                &f.name,
                &fixtures::arguments(f),
            )
            .unwrap();
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
    assert_eq!(actions, 18);
}

#[test]
fn cli_exports_hierarchy_and_rejects_stale_metadata() {
    let path = sibling::isa().join("reference-interpreter/tests/fixtures/control_flow.bisa");
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
    let fixture = sibling::isa().join("reference-interpreter/tests/fixtures/control_flow.bisa");
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

#[test]
fn write_api_preserves_legacy_roundtrip_without_circuit_body_clones() {
    let object = Object::parse(
        &std::fs::read_to_string(
            sibling::isa().join("reference-interpreter/tests/fixtures/control_flow.bisa"),
        )
        .unwrap(),
    )
    .unwrap();
    let program =
        bennett_circuits::lower::lower_program_with_args(&object, "controlled_phase", &[]).unwrap();
    let mut bytes = vec![];
    let stats = qasm::write(&program, &mut bytes).unwrap();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        qasm::export(&program).unwrap()
    );
    assert_eq!(
        qasm::parse(std::str::from_utf8(&bytes).unwrap()).unwrap(),
        program
    );
    assert_eq!(stats.circuit_body_clones, 0);
    assert_eq!(stats.per_gate_program_scans, 0);
}

struct FailingWriter {
    bytes: Vec<u8>,
    limit: usize,
    kind: std::io::ErrorKind,
    flush_error: bool,
    short: usize,
}
impl std::io::Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len() >= self.limit {
            return Err(std::io::Error::new(
                self.kind,
                "injected output/allocation failure",
            ));
        }
        let count = bytes
            .len()
            .min(self.limit - self.bytes.len())
            .min(self.short);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.flush_error {
            Err(std::io::Error::other("injected flush failure"))
        } else {
            Ok(())
        }
    }
}
fn small_stream() -> (Object, bennett_circuits::oracle_emit::Recipes) {
    let obj = Object::parse(include_str!("fixtures/export_oracle/increment2.bennett")).unwrap();
    let b = bennett_circuits::oracle::Budget {
        steps: 3,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    let recipes = bennett_circuits::oracle_emit::prepare(&obj, "inc2", b, &[])
        .unwrap()
        .unwrap();
    (obj, recipes)
}
#[test]
fn stream_io_allocation_short_write_and_flush_failures_are_explicit() {
    let (obj, recipes) = small_stream();
    let mut complete = vec![];
    qasm::write_oracle(&obj, &recipes, &mut complete).unwrap();
    for kind in [
        std::io::ErrorKind::Other,
        std::io::ErrorKind::OutOfMemory,
        std::io::ErrorKind::WriteZero,
    ] {
        for limit in [0, 80, complete.len() / 2, complete.len() - 3] {
            let mut sink = FailingWriter {
                bytes: vec![],
                limit,
                kind,
                flush_error: false,
                short: 11,
            };
            assert!(qasm::write_oracle(&obj, &recipes, &mut sink).is_err());
            assert!(sink.bytes.len() <= limit);
        }
    }
    let mut sink = FailingWriter {
        bytes: vec![],
        limit: usize::MAX,
        kind: std::io::ErrorKind::Other,
        flush_error: false,
        short: 3,
    };
    let stats = qasm::write_oracle(&obj, &recipes, &mut sink).unwrap();
    assert_eq!(sink.bytes, complete);
    assert_eq!(stats.bytes, complete.len() as u128);
    sink = FailingWriter {
        bytes: vec![],
        limit: usize::MAX,
        kind: std::io::ErrorKind::Other,
        flush_error: true,
        short: usize::MAX,
    };
    assert!(
        qasm::write_oracle(&obj, &recipes, &mut sink)
            .unwrap_err()
            .0
            .contains("flush")
    );
    let mut wrong = String::from_utf8(obj.text().as_bytes().to_vec()).unwrap();
    wrong = wrong.replace("const.i2 1", "const.i2 2");
    let wrong = Object::parse(&wrong).unwrap();
    let mut untouched = vec![];
    assert!(qasm::write_oracle(&wrong, &recipes, &mut untouched).is_err());
    assert!(untouched.is_empty());
}
#[test]
fn checked_stream_rejects_truncation_metadata_and_gate_edits_independently() {
    let (obj, recipes) = small_stream();
    let mut bytes = vec![];
    qasm::write_oracle(&obj, &recipes, &mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for bad in [
        text.replace("bc-oracle-stream-1", "bc-oracle-stream-2"),
        text.replacen("x q[", "h q[", 1),
        format!("{text}x q[0];\n"),
        text.lines()
            .filter(|l| !l.starts_with("// bc-end:"))
            .collect::<Vec<_>>()
            .join("\n"),
        text[..text.len() - 3].to_string(),
    ] {
        assert!(qasm::parse(&bad).is_err());
        assert!(qasm::visit_stream_checked(std::io::Cursor::new(&bad), |_| Ok(())).is_err());
    }
    let changed = text.replacen("x q[", "h q[", 1);
    let mut saw_h = false;
    qasm::visit_stream(std::io::Cursor::new(changed), |g| {
        saw_h |= g.gate == bennett_circuits::ir::BaseGate::H;
        Ok(())
    })
    .unwrap();
    assert!(saw_h);
    let executable = text
        .lines()
        .filter(|l| !l.starts_with("// bc-"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        qasm::parse_flat(&executable).unwrap().gates.len() as u128,
        recipes.counts().full
    );
    let bad = executable.replacen("q[0]", "q[999999]", 1);
    assert!(qasm::parse_flat(&bad).is_err());
    let duplicate = text.replacen(
        "// interface:",
        &format!("// interface: {}\n// interface:", recipes.interface_wires()),
        1,
    );
    assert!(qasm::parse(&duplicate).is_err());
}
#[test]
fn atomic_output_preserves_previous_file_on_failure_and_removes_partial_files() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/qasm-atomic-test");
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("result.qasm");
    std::fs::write(&target, b"previous complete output").unwrap();
    let result = qasm::write_atomic(&target, |out| {
        out.write_all(b"incomplete").unwrap();
        Err(bennett_circuits::Error("injected failure".into()))
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"previous complete output");
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".partial-")
    }));
    qasm::write_atomic(&target, |out| {
        out.write_all(b"new complete output")
            .map_err(|e| bennett_circuits::Error(e.to_string()))
    })
    .unwrap();
    assert_eq!(std::fs::read(target).unwrap(), b"new complete output");
}
#[test]
fn indexed_writer_validates_before_output_and_streams_large_primitive_bodies() {
    use bennett_circuits::ir::*;
    let gate = Node::Gate(GateInst {
        gate: BaseGate::X,
        own_controls: 0,
        controls: vec![],
        targets: vec![QubitRef::new("q", 0)],
    });
    let c = Circuit {
        name: "big".into(),
        perm: true,
        mut_bundles: vec![Bundle {
            name: "q".into(),
            width: 1,
            role: Role::Mut,
        }],
        in_bundles: vec![],
        mem: vec![],
        workspace: vec![],
        local_regions: vec![],
        body: vec![gate; 10_000],
    };
    let mut p = Program {
        top: "big".into(),
        circuits: vec![c],
    };
    let mut sink = std::io::sink();
    let stats = qasm::write(&p, &mut sink).unwrap();
    assert_eq!(stats.writer_visits, 10_000);
    assert_eq!(stats.peak_gate_objects, 1);
    assert_eq!(stats.circuit_body_clones, 0);
    assert_eq!(stats.per_gate_program_scans, 0);
    p.circuits[0].body.push(Node::Call {
        callee: "missing".into(),
        adjoint: false,
        controls: vec![],
        args: vec![],
    });
    let mut bytes = vec![];
    assert!(qasm::write(&p, &mut bytes).is_err());
    assert!(bytes.is_empty());
}

#[test]
fn malformed_compact_annotations_fail_before_visiting_any_operations() {
    use bennett_circuits::json::Value;
    let (object, recipes) = small_stream();
    let mut bytes = vec![];
    qasm::write_oracle(&object, &recipes, &mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let original = text
        .lines()
        .find_map(|line| line.strip_prefix("// bc-stream: "))
        .unwrap();
    for bad_argument in [false, true] {
        let mut value = Value::parse(original).unwrap();
        let Value::Object(ref mut top) = value else {
            unreachable!()
        };
        if bad_argument {
            let Value::Object(binding) = top.get_mut("binding").unwrap() else {
                unreachable!()
            };
            binding.insert(
                "arguments".into(),
                Value::parse("[[\"integer\",\"--2\"]]").unwrap(),
            );
        } else {
            let Value::Object(counts) = top.get_mut("counts").unwrap() else {
                unreachable!()
            };
            counts.insert("descriptors".into(), Value::Str("not-a-number".into()));
        }
        let bad = text.replacen(original, &value.to_jcs(), 1);
        let mut visited = 0;
        assert!(
            qasm::visit_stream_checked(std::io::Cursor::new(bad), |_| {
                visited += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(visited, 0);
    }
    let raw="OPENQASM 3.0;\ninclude \"stdgates.inc\";\n// interface: 2\nqubit[2] q;\nctrl @ inv @ s q[0], q[1];\n".replace("\\n","\n").replace("\\\"","\"");
    let parsed = qasm::parse_flat(&raw).unwrap();
    assert_eq!(parsed.gates[0].gate, bennett_circuits::ir::BaseGate::Sdg);
}
