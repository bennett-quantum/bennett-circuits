use bennett_circuits::{
    json::Value,
    linking::Registry,
    object::{Object, Symbol, derived_signature},
};
fn object(body: &str) -> Object {
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
    o
}
#[path = "support/target.rs"]
mod targets;
fn target() -> bennett_circuits::target::Target {
    targets::universal(false, false)
}
fn context(t: &bennett_circuits::target::Target) -> bennett_circuits::certificate::Context<'_> {
    bennett_circuits::certificate::Context {
        target: t,
        dialect_mapping_version: "direct-isa-0.8",
    }
}
fn link(root: &Object, libraries: &[Object]) -> bennett_circuits::Result<Registry> {
    let root = root.materialize()?;
    let libraries = libraries
        .iter()
        .map(Object::materialize)
        .collect::<bennett_circuits::Result<Vec<_>>>()?;
    Registry::link(&root, &libraries)
}
#[test]
fn two_level_linking_discharges_and_rebinds() {
    let leaf = object("perm_action @leaf (x:wire mut) () () () ()\n^entry():\nx x\n");
    let mut middle = object(
        "perm_action @middle (x:wire mut) () () () ()\n^entry():\naction_call @leaf (x) () () ()\n",
    );
    middle.imports.push(bennett_circuits::object::Import {
        name: "leaf".into(),
        signature: leaf.symbols[0].signature.clone(),
        pinned_closure_hash: None,
    });
    let mut root = object(
        "perm_action @root (x:wire mut) () () () ()\n^entry():\naction_call @middle (x) () () ()\n",
    );
    root.imports.push(bennett_circuits::object::Import {
        name: "middle".into(),
        signature: middle.symbols[0].signature.clone(),
        pinned_closure_hash: None,
    });
    let r = link(&root, &[middle.clone(), leaf.clone()]).unwrap();
    assert_eq!(r.object.check().unwrap().proof_status, "proved");
    assert!(
        r.resolve(
            &r.object.hash().unwrap().symbols["root"]
                .closure_hash
                .clone()
                .unwrap()
        )
        .is_ok()
    );
    middle.imports[0].signature = Value::parse(r#"{"kind":"invalid"}"#).unwrap();
    assert!(link(&root, &[middle, leaf]).is_err());
}
#[test]
fn linked_body_must_discharge_pending_cleanup() {
    let leaf = object("action @leaf (x:wire mut) () () () ()\n^entry():\nh x\n");
    let mut root = object(
        "action @root (x:wire mut) () () () ()\n^entry():\nd = qcopy x\naction_call @leaf (x) () () ()\nquncopy x d\n",
    );
    root.imports.push(bennett_circuits::object::Import {
        name: "leaf".into(),
        signature: leaf.symbols[0].signature.clone(),
        pinned_closure_hash: None,
    });
    assert_eq!(root.hash().unwrap().proof_status, "pending");
    assert!(link(&root, &[leaf]).unwrap_err().0.contains("§6.2"));
}
#[test]
fn draft_frame_requires_opt_in_and_rejects_unknown_fields() {
    use bennett_circuits::frame::Frame;
    let v=Value::parse(r#"{"ancillae":[{"class":"fast","id":"a","scheme":"controlled-x","state":"clean","width":{"var":"c"}}],"borrow":[{"class":"fast","id":"b","width":1}],"history":[{"class":"fast","id":"h","width":3}],"live_ranges":[{"end":2,"frame":"s","start":0},{"end":4,"frame":"s","start":2}],"scratch":[{"class":"fast","id":"s","state":"clean","width":{"args":[2,{"var":"c"}],"op":"+"}}],"slots":[{"class":"fast","id":"mut:0","role":"mut","width":2},{"class":"fast","id":"in:0","role":"in","width":1}]}"#).unwrap();
    assert_eq!(
        bennett_circuits::sha256::sha256_hex(v.to_jcs().as_bytes()),
        "f314eeef24a5a65a9c72f91e062aa3c83556358eaca166e689571700a72083ff"
    );
    assert!(Frame::parse(&v, false).unwrap_err().0.contains("0.9-draft"));
    let f = Frame::parse(&v, true).unwrap();
    assert_eq!(f.to_json(), v);
    let mut bad = v.clone();
    if let Value::Object(m) = &mut bad {
        m.insert("unknown".into(), Value::Array(vec![]));
    }
    assert!(Frame::parse(&bad, true).is_err());
    assert!(
        Frame::parse(
            &Value::parse(&v.to_jcs().replace("\"c\"", "\"k\"")).unwrap(),
            true
        )
        .is_err()
    );
    assert!(
        Frame::parse(
            &Value::parse(&v.to_jcs().replace("\"start\":2", "\"start\":1")).unwrap(),
            true
        )
        .is_err()
    );
}
#[test]
fn full_tuple_certificate_roundtrip_and_tamper_rejection() {
    use bennett_circuits::certificate;
    let o = object("perm_action @a (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    assert!(
        certificate::generate(&o, "a", &[], None, false, &context(&target()))
            .unwrap_err()
            .0
            .contains("0.9-draft")
    );
    let cert = certificate::generate(&o, "a", &[], None, true, &context(&target())).unwrap();
    let claims = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .get("claims")
        .unwrap()
        .as_array()
        .unwrap();
    let rows = claims[0]
        .get("expected")
        .unwrap()
        .get("table")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0]
            .get("before")
            .unwrap()
            .get("bundles")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_str(),
        Some("0")
    );
    let mut certified = o.clone();
    certified.draft_encodings = true;
    certified.certificate = Some(cert.clone());
    let roundtrip = certified.materialize().unwrap();
    roundtrip.check().unwrap();
    assert!(Object::parse(roundtrip.text()).unwrap().check().is_err());
    let bad = Value::parse(&cert.to_jcs().replace(
        "\"after\":{\"borrowed\":[],\"bundles\":[\"1\"]",
        "\"after\":{\"borrowed\":[],\"bundles\":[\"0\"]",
    ))
    .unwrap();
    certified.certificate = Some(bad);
    assert!(certified.check().is_err());
    let bad = Value::parse(&cert.to_jcs().replace(
        ",\"invocation\":{\"args\":[],\"budget\":[\"0\",\"0\",\"0\",\"0\"]}",
        "",
    ))
    .unwrap();
    certified.certificate = Some(bad);
    assert!(certified.check().is_err());
}
#[test]
fn full_domain_borrow_factorisation_is_not_a_zero_borrow_test() {
    let o=object("perm_action @a (x:wire mut) () () () ()\n^entry():\nb = qborrow any 1\nswap b x\nswap b x\nqreturn b\n").materialize().unwrap();
    let cert =
        bennett_circuits::certificate::generate(&o, "a", &[], None, true, &context(&target()))
            .unwrap();
    let rows = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .get("claims")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .get("expected")
        .unwrap()
        .get("table")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[1]
            .get("before")
            .unwrap()
            .get("borrowed")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_str(),
        Some("1")
    );
    let bad=object("perm_action @a (x:wire mut) () () () ()\n^entry():\nb = qborrow any 1\ncx b x\nqreturn b\n").materialize().unwrap();
    assert!(
        bennett_circuits::certificate::generate(&bad, "a", &[], None, true, &context(&target()))
            .unwrap_err()
            .0
            .contains("§5.6")
    );
}
#[test]
fn linking_keeps_and_rechecks_library_certificates() {
    let mut leaf = object("perm_action @leaf (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    leaf.draft_encodings = true;
    leaf.certificate = Some(
        bennett_circuits::certificate::generate(
            &leaf,
            "leaf",
            &[],
            None,
            true,
            &context(&target()),
        )
        .unwrap(),
    );
    let mut root = object(
        "perm_action @root (x:wire mut) () () () ()\n^entry():\naction_call @leaf (x) () () ()\n",
    );
    root.imports.push(bennett_circuits::object::Import {
        name: "leaf".into(),
        signature: leaf.symbols[0].signature.clone(),
        pinned_closure_hash: None,
    });
    let registry = link(&root, &[leaf]).unwrap();
    assert!(registry.object.certificate.is_some());
    registry.object.check().unwrap();
}
#[test]
fn certificates_replayed_against_reference_cli() {
    use std::process::Command;
    let binary = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref";
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/m4-reference");
    std::fs::create_dir_all(&dir).unwrap();
    let o = object("perm_action @a (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    let source = dir.join("source.bisa");
    std::fs::write(&source, o.text()).unwrap();
    for command in ["check", "hash"] {
        let output = Command::new(binary)
            .arg(command)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = Value::parse(std::str::from_utf8(&output.stdout).unwrap().trim()).unwrap();
        assert_eq!(
            report.get("object_hash").unwrap().as_str(),
            Some(o.object_hash().as_str())
        );
    }
    let cert =
        bennett_circuits::certificate::generate(&o, "a", &[], None, true, &context(&target()))
            .unwrap();
    let claims = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .get("claims")
        .unwrap()
        .as_array()
        .unwrap();
    let rows = claims[0]
        .get("expected")
        .unwrap()
        .get("table")
        .unwrap()
        .as_array()
        .unwrap();
    for row in rows {
        let before = row
            .get("before")
            .unwrap()
            .get("bundles")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_str()
            .unwrap();
        let after = row
            .get("after")
            .unwrap()
            .get("bundles")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .as_str()
            .unwrap();
        let output = Command::new(binary)
            .arg("run-action")
            .arg(&source)
            .args(["a", "--basis", before])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report =
            Value::parse_target(std::str::from_utf8(&output.stdout).unwrap().trim()).unwrap();
        assert_eq!(
            report.get("state").unwrap().get(after).unwrap(),
            &Value::Array(vec![Value::Int("1".into()), Value::Int("0".into())])
        );
    }
    let mut certified = o;
    certified.draft_encodings = true;
    certified.certificate = Some(cert);
    let path = dir.join("certified.bisa");
    std::fs::write(&path, certified.materialize().unwrap().text()).unwrap();
    for command in ["check", "hash"] {
        let output = Command::new(binary)
            .arg(command)
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("M4 DRAFT DIFFERENTIAL {command}: {stderr}");
        assert!(!output.status.success());
        assert!(stderr.contains("certificate version mismatch"), "{stderr}");
    }
}
#[test]
fn coverage_records_every_trap_and_requires_real_source_mapping() {
    let mut o = object("oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nfault 0\n")
        .materialize()
        .unwrap();
    assert!(bennett_circuits::coverage::traps(&o, &Default::default()).is_err());
    let h = o.hash().unwrap().symbols["o"].interface_hash.clone();
    let key = Value::Array(vec![
        Value::Str(h),
        Value::Str("entry".into()),
        Value::Int("0".into()),
    ])
    .to_jcs();
    let sources = std::collections::BTreeMap::from([(key, "sem-fault".into())]);
    o.coverage = Some(bennett_circuits::coverage::traps(&o, &sources).unwrap());
    o.check().unwrap();
    let entries = o
        .coverage
        .as_ref()
        .unwrap()
        .get("entries")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].get("fault_code").unwrap().as_str(), Some("4"));
    let emitted = o.materialize().unwrap();
    assert_eq!(emitted.coverage, o.coverage);
}
#[test]
fn inverse_pairs_bind_full_domain_budget_and_both_orientations() {
    use bennett_circuits::{certificate, oracle::Budget};
    let o=object("oracle @g (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\noracle @gi (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\nperm_action @owner (x:wire mut) () () () ()\n^entry():\nx x\n").materialize().unwrap();
    let mut cert =
        certificate::generate(&o, "owner", &[], None, true, &context(&target())).unwrap();
    let budget = Budget {
        steps: 1,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    certificate::add_inverse_pair(&o, &mut cert, "owner", "g", "gi", budget).unwrap();
    let mut bound = o.clone();
    bound.draft_encodings = true;
    bound.certificate = Some(cert);
    bound.check().unwrap();
    let mut cert =
        certificate::generate(&o, "owner", &[], None, true, &context(&target())).unwrap();
    assert!(
        certificate::add_inverse_pair(
            &o,
            &mut cert,
            "owner",
            "g",
            "gi",
            Budget { steps: 0, ..budget }
        )
        .is_err()
    );
}
fn region_mut(cert: &mut Value) -> &mut std::collections::BTreeMap<String, Value> {
    let Value::Object(top) = cert else { panic!() };
    let Value::Object(regions) = top.get_mut("regions").unwrap() else {
        panic!()
    };
    let Value::Object(region) = regions.values_mut().next().unwrap() else {
        panic!()
    };
    region
}
#[test]
fn library_claim_dag_rejects_cycles_and_rebinds_results() {
    use bennett_circuits::{certificate, oracle::Budget};
    let o=object("oracle @g (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\noracle @gi (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\nperm_action @root (x:wire mut) () () () ()\n^entry():\nx x\nperm_action @middle (x:wire mut) () () () ()\n^entry():\nx x\nx x\nperm_action @library (x:wire mut) () () () ()\n^entry():\nx x\nx x\nx x\n").materialize().unwrap();
    let ids = o.hash().unwrap().symbols;
    let hash = |name: &str| ids[name].closure_hash.as_ref().unwrap().clone();
    let budget = Budget {
        steps: 1,
        heap: 0,
        stack: 0,
        depth: 0,
    };
    let mut certs = vec![];
    for name in ["root", "middle", "library"] {
        let mut cert =
            certificate::generate(&o, name, &[], None, true, &context(&target())).unwrap();
        certificate::add_inverse_pair(&o, &mut cert, name, "g", "gi", budget).unwrap();
        certs.push(cert);
    }
    for (i, callee) in [(0, "middle"), (1, "library")] {
        let r = region_mut(&mut certs[i]);
        let Value::Array(claims) = r.get_mut("claims").unwrap() else {
            panic!()
        };
        claims[2]=Value::parse(&format!(r#"{{"kind":"structural","rule":"inverse_pair_library","subclaims":[],"witness":{{"library_closure_hash":"{}","claim_index":2}},"invocation":{{"args":"all","budget":"all"}}}}"#,hash(callee))).unwrap();
    }
    let mut combined = certs.remove(0);
    for cert in certs {
        let records = cert.get("regions").unwrap().as_object().unwrap().clone();
        let Value::Object(top) = &mut combined else {
            panic!()
        };
        let Value::Object(regions) = top.get_mut("regions").unwrap() else {
            panic!()
        };
        regions.extend(records);
    }
    let mut bound = o.clone();
    bound.draft_encodings = true;
    bound.certificate = Some(combined.clone());
    bound.check().unwrap();
    let cyclic = Value::parse(&combined.to_jcs().replace(
        &format!("\"library_closure_hash\":\"{}\"", hash("library")),
        &format!("\"library_closure_hash\":\"{}\"", hash("root")),
    ))
    .unwrap();
    bound.certificate = Some(cyclic);
    assert!(bound.check().unwrap_err().0.contains("cyclic"));
    let wrong = Value::parse(
        &combined
            .to_jcs()
            .replace("\"claim_index\":2", "\"claim_index\":1"),
    )
    .unwrap();
    bound.certificate = Some(wrong);
    assert!(
        bound
            .check()
            .unwrap_err()
            .0
            .contains("not an inverse-pair proof")
    );
}
#[test]
fn symbolic_histories_include_control_contexts_and_text_order_blocks() {
    let body = "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (t:wire mut, fr:wire[4] mut) (x:wire in, p:wire in) () (tk:token) ()\n^first():\nqpred p\nd = qcopy x\noracle_call @o tk (x) t fr () ()\n^second():\nquncopy x d\nqunpred p\n";
    object(body).hash().unwrap();
    let bad = body.replace("quncopy x d\nqunpred p", "qunpred p\nquncopy x d");
    assert!(object(&bad).hash().unwrap_err().0.contains("§6.2"));
}
#[test]
fn translation_witnesses_bind_source_points_and_wire_sets() {
    let o=object("perm_action @a (y:wire mut) (x:wire in) () () ()\n^entry():\nb = qborrow any 1\ncx x b\ncx b y\ncx x b\ncx b y\nqreturn b\n").materialize().unwrap();
    let cert =
        bennett_circuits::certificate::generate(&o, "a", &[], None, true, &context(&target()))
            .unwrap();
    let claims = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .get("claims")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(
        claims
            .iter()
            .any(|c| c.get("rule").and_then(Value::as_str) == Some("translation_lemma"))
    );
    let mut bound = o;
    bound.draft_encodings = true;
    bound.certificate =
        Some(Value::parse(&cert.to_jcs().replace("\"X\":[\"x\"]", "\"X\":[\"y\"]")).unwrap());
    assert!(bound.check().is_err());
}
#[test]
fn symbolic_histories_never_erase_quantum_use_or_merge_distinct_tokens() {
    let prefix = "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\n";
    let good = format!(
        "{prefix}action @a () (x:wire in) () (tk:token, other:token) ()\n^entry():\nt = qalloc any 1\nf = qalloc any 4\noracle_call @o tk (x) t f () ()\noracle_call @o tk (x) t f () ()\nqfree t\nqfree f\n"
    );
    object(&good).hash().unwrap();
    let bad = good.replacen(
        "oracle_call @o tk (x) t f () ()\nqfree",
        "oracle_call @o other (x) t f () ()\nqfree",
        1,
    );
    assert!(object(&bad).hash().unwrap_err().0.contains("§6.2"));
    let bad = format!(
        "{prefix}action @a (t:wire mut, fr:wire[4] mut) () () (tk:token) ()\n^entry():\nd = qalloc any 1\nh d\noracle_call @o tk (d) t fr () ()\nh d\nqfree d\n"
    );
    assert!(object(&bad).hash().unwrap_err().0.contains("§6.2"));
}
#[test]
fn placed_frame_emits_and_parses_the_same_typed_records() {
    let o = object("perm_action @a (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    let p = bennett_circuits::placement::place_on_target(&o, "a", &[], &target()).unwrap();
    assert!(bennett_circuits::frame::placed(&p, &o, false).is_err());
    let frame = bennett_circuits::frame::placed(&p, &o, true).unwrap();
    let emitted = bennett_circuits::qasm::export_placed_draft(&p, &o).unwrap();
    let metadata = emitted
        .lines()
        .find_map(|l| l.strip_prefix("// bc-placement: "))
        .unwrap();
    let metadata = Value::parse(metadata).unwrap();
    assert_eq!(
        bennett_circuits::frame::Frame::parse(metadata.get("frame").unwrap(), true).unwrap(),
        frame
    );
    let text = frame.to_json().to_jcs();
    assert_eq!(
        bennett_circuits::frame::Frame::parse(&Value::parse(&text).unwrap(), true).unwrap(),
        frame
    );
}
#[test]
fn linking_deduplicates_equal_closures_and_rejects_unequal_definitions() {
    let a = object(
        "perm_action @left (x:wire mut) () () () ()\n^entry():\nx x\nperm_action @same (x:wire mut) () () () ()\n^entry():\naction_call @left (x) () () ()\n",
    );
    let b = object(
        "perm_action @right (x:wire mut) () () () ()\n^entry():\nx x\nperm_action @same (x:wire mut) () () () ()\n^entry():\naction_call @right (x) () () ()\n",
    );
    link(&a, std::slice::from_ref(&a)).unwrap();
    // §9.2 includes the callee definition header in canonical bytes: changing
    // @left to @right does not establish equal closure identity.
    assert!(link(&a, &[b]).unwrap_err().0.contains("unequal closure"));
}
#[test]
fn symbolic_nonpermutation_calls_close_read_only_dependency_windows() {
    let o = object(
        "action @phase () (x:wire in) () () ()\n^entry():\ns x\naction @a () (x:wire in) () (unused:token) ()\n^entry():\nd = qcopy x\naction_call @phase () (x) () ()\nquncopy x d\n",
    );
    assert!(o.hash().unwrap_err().0.contains("§6.2"));
}
#[test]
fn legacy_numerical_claims_cannot_bypass_the_expansion_library_gate() {
    let mut o = object("perm_action @a (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    let mut cert =
        bennett_circuits::certificate::generate(&o, "a", &[], None, true, &context(&target()))
            .unwrap();
    let h = o.hash().unwrap().symbols["a"].closure_hash.clone().unwrap();
    let r = region_mut(&mut cert);
    r.remove("frame");
    r.insert("verification".into(), Value::parse("{}").unwrap());
    r.insert("claims".into(),Value::parse(&format!(r#"[{{"kind":"numerical","subject":{{"closure_hash":"{h}","variant":"forward","inherited_controls":0,"schedule_hash":"{}"}},"metric":"operator_norm","precision_bits":64,"bound":"0"}}]"#,"0".repeat(64))).unwrap());
    let Value::Object(top) = &mut cert else {
        panic!()
    };
    let Value::Object(versions) = top.get_mut("versions").unwrap() else {
        panic!()
    };
    versions.insert("cert".into(), Value::Str("0.8".into()));
    o.certificate = Some(cert);
    assert!(o.check().unwrap_err().0.contains("b-isa-spec-8r4"));
}
#[test]
fn action_frames_preserve_nested_oracle_history_provenance() {
    use bennett_circuits::oracle::{Argument, Budget};
    let o=object("oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (t:wire mut, fr:wire[4] mut) (x:wire in) () (tk:token) ()\n^entry():\noracle_call @o tk (x) t fr () ()\n").materialize().unwrap();
    let args = [Argument::Token(Budget {
        steps: 1,
        heap: 0,
        stack: 0,
        depth: 0,
    })];
    let cert =
        bennett_circuits::certificate::generate(&o, "a", &args, None, true, &context(&target()))
            .unwrap();
    let region = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert!(
        !region
            .get("frame")
            .unwrap()
            .get("history")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let p = bennett_circuits::oracle::specialise(
        &o,
        "o",
        Budget {
            steps: 1,
            heap: 0,
            stack: 0,
            depth: 0,
        },
        &[],
    )
    .unwrap();
    assert!(
        p.circuits[0]
            .workspace
            .iter()
            .any(|w| w.kind == bennett_circuits::ir::WsKind::RetainedHistory)
    );
}
#[test]
fn compact_program_relations_generate_without_materialising_tuple_rows() {
    let o = object("perm_action @a (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    let h = o.hash().unwrap().symbols["a"].closure_hash.clone().unwrap();
    let cert = bennett_circuits::certificate::generate_program(
        &o,
        "a",
        &[],
        None,
        true,
        &context(&target()),
        &h,
    )
    .unwrap();
    let claims = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .get("claims")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        claims[0]
            .get("expected")
            .unwrap()
            .get("program")
            .unwrap()
            .as_str(),
        Some(h.as_str())
    );
    assert!(claims[0].get("expected").unwrap().get("table").is_none());
    let mut bound = o;
    bound.draft_encodings = true;
    bound.certificate = Some(cert);
    bound.check().unwrap();
}
#[test]
fn oracle_tables_cover_arbitrary_target_and_fault_in_signature_order() {
    use bennett_circuits::oracle::Budget;
    let o = object("oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\n")
        .materialize()
        .unwrap();
    let cert = bennett_circuits::certificate::generate(
        &o,
        "o",
        &[],
        Some(Budget {
            steps: 1,
            heap: 0,
            stack: 0,
            depth: 0,
        }),
        true,
        &context(&target()),
    )
    .unwrap();
    let region = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let slots = region
        .get("frame")
        .unwrap()
        .get("slots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        slots
            .iter()
            .map(|s| s.get("id").unwrap().as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["in:0", "tgt:0", "fault:0"]
    );
    let rows = region.get("claims").unwrap().as_array().unwrap()[0]
        .get("expected")
        .unwrap()
        .get("table")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 64);
    let last = rows.last().unwrap();
    assert_eq!(
        last.get("before").unwrap().get("bundles").unwrap(),
        &Value::parse(r#"["1","1","15"]"#).unwrap()
    );
    assert_eq!(
        last.get("after").unwrap().get("bundles").unwrap(),
        &Value::parse(r#"["1","0","15"]"#).unwrap()
    );
    let mut bound = o;
    bound.draft_encodings = true;
    bound.certificate = Some(cert);
    bound.check().unwrap();
}
#[test]
fn histories_reject_noncall_modifiers_and_closed_control_dependencies() {
    let bad = object("action @a (p:wire mut) () () (unused:token) ()\n^entry():\nctl p x p\n");
    assert!(bad.hash().is_err());
    let bad = object(
        "action @a (p:wire mut) () () (unused:token) ()\n^entry():\nqpred p\nd = qalloc any 1\nh d\nqunpred p\nh p\nh p\nqpred p\nh d\nqfree d\nqunpred p\n",
    );
    assert!(bad.hash().unwrap_err().0.contains("§6.2"));
}
#[test]
fn history_versions_cover_predicated_copies_and_oracle_reads() {
    let bad = object(
        "action @a (p:wire mut) (x:wire in) () (unused:token) ()\n^entry():\nqpred p\nd = qcopy x\nqunpred p\nh p\nh p\nqpred p\nquncopy x d\nqunpred p\n",
    );
    assert!(bad.hash().unwrap_err().0.contains("§6.2"));
    let bad = object(
        "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (x:wire mut) () () (tk:token) ()\n^entry():\nt = qalloc any 1\nf = qalloc any 4\noracle_call @o tk (x) t f () ()\nh x\nh x\noracle_call @o tk (x) t f () ()\nqfree t\nqfree f\n",
    );
    assert!(bad.hash().unwrap_err().0.contains("§6.2"));
}
#[test]
fn history_rows_enforce_alias_class_result_and_active_selector_schemas() {
    for body in [
        "d = qcopy x\nquncopy d d\n",
        "d = qalloc 7 1\nqfree d\n",
        "r = qxor x y\n",
        "p = qalloc any 1\nqpred p\nqfree p\nqunpred p\n",
    ] {
        let source = format!(
            "action @a (x:wire mut, y:wire mut) () () (unused:token) ()\n^entry():\n{body}"
        );
        assert!(
            object(&source).hash().is_err(),
            "accepted malformed row: {body}"
        );
    }
}
#[test]
fn symbolic_load_histories_span_calls_and_keep_region_versions() {
    let body = "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (t:wire mut, fr:wire[4] mut) (x:wire in, addr:wire in) (r:region[1,1] in) (tk:token) ()\n^entry():\nd = qload r addr\noracle_call @o tk (x) t fr () ()\nqunload r addr d\n";
    object(body).hash().unwrap();
    let bad = body.replace("qunload r addr d", "qunload r x d");
    assert!(object(&bad).hash().unwrap_err().0.contains("§6.2"));
}
#[test]
fn memory_versions_close_across_mutable_calls_even_after_their_adjoint() {
    let prefix = "perm_action @change () (a:wire in, v:wire in) (r:region[1,1] mut) () ()\n^entry():\nqstore r a v\n";
    let before = format!(
        "{prefix}action @a () (a:wire in, v:wire in) (r:region[1,1] mut) (unused:token) ()\n^entry():\nd = qload r a\naction_call @change () (a, v) (r) ()\nadj action_call @change () (a, v) (r) ()\nqunload r a d\n"
    );
    assert!(object(&before).hash().unwrap_err().0.contains("§6.2"));
    let during = format!(
        "{prefix}action @a () (a:wire in, v:wire in) (r:region[1,1] mut) (unused:token) ()\n^entry():\naction_call @change () (a, v) (r) ()\nd = qload r a\nqunload r a d\nadj action_call @change () (a, v) (r) ()\n"
    );
    object(&during).hash().unwrap();
    let bad = during.replace("qunload r a d", "e = qcopy d\nqunload r a e\nqunload r a d");
    assert!(object(&bad).hash().unwrap_err().0.contains("§6.2"));
}
#[test]
fn every_oracle_certificate_row_replays_with_arbitrary_targets_and_faults() {
    use bennett_circuits::oracle::Budget;
    use std::process::Command;
    let o=object("oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @probe (t:wire mut, fr:wire[4] mut) (x:wire in) () (tk:token) ()\n^entry():\noracle_call @o tk (x) t fr () ()\n").materialize().unwrap();
    let cert = bennett_circuits::certificate::generate(
        &o,
        "o",
        &[],
        Some(Budget {
            steps: 1,
            heap: 0,
            stack: 0,
            depth: 0,
        }),
        true,
        &context(&target()),
    )
    .unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/m4-oracle-rows");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("source.bisa");
    std::fs::write(&path, o.text()).unwrap();
    let binary = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref";
    for command in ["check", "hash"] {
        let output = Command::new(binary)
            .arg(command)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = Value::parse(std::str::from_utf8(&output.stdout).unwrap().trim()).unwrap();
        assert_eq!(
            report.get("object_hash").unwrap().as_str(),
            Some(o.object_hash().as_str())
        );
    }
    let row_bits = |tuple: &Value| {
        let a = tuple
            .get("bundles")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().parse::<u8>().unwrap())
            .collect::<Vec<_>>();
        format!(
            "{}{}{}{}{}{}",
            a[1],
            a[2] & 1,
            (a[2] >> 1) & 1,
            (a[2] >> 2) & 1,
            (a[2] >> 3) & 1,
            a[0]
        )
    };
    let claims = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .get("claims")
        .unwrap()
        .as_array()
        .unwrap();
    let rows = claims[0]
        .get("expected")
        .unwrap()
        .get("table")
        .unwrap()
        .as_array()
        .unwrap();
    for row in rows {
        let before = row_bits(row.get("before").unwrap());
        let after = row_bits(row.get("after").unwrap());
        let output = Command::new(binary)
            .arg("run-action")
            .arg(&path)
            .args(["probe", "--basis", &before, "--arg", "token:1,0,0,0"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report =
            Value::parse_target(std::str::from_utf8(&output.stdout).unwrap().trim()).unwrap();
        let state = report.get("state").unwrap().as_object().unwrap();
        assert_eq!(state.len(), 1);
        assert_eq!(state[&after], Value::parse("[1,0]").unwrap());
    }
    let mut bound = o;
    bound.draft_encodings = true;
    bound.certificate = Some(cert);
    let path = dir.join("certified.bisa");
    std::fs::write(&path, bound.materialize().unwrap().text()).unwrap();
    for command in ["check", "hash"] {
        let output = Command::new(binary)
            .arg(command)
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("certificate version mismatch"));
    }
    eprintln!(
        "M4 ORACLE CERTIFICATE: 64 full tuples agree, including arbitrary target/fault; draft check/hash stop at version boundary"
    );
}
#[test]
fn existing_exact_quantum_cancellation_is_preserved_with_unused_tokens() {
    let o = object(
        "action @phase () (x:wire in) () () ()\n^entry():\ns x\naction @a () () () (unused:token) ()\n^entry():\nd = qalloc any 1\nh d\naction_call @phase () (d) () ()\nadj action_call @phase () (d) () ()\nh d\nqfree d\n",
    );
    o.hash().unwrap();
}
#[test]
fn program_relation_compatibility_uses_operand_contracts_not_permutation_markers() {
    let o=object("action @a (x:wire mut) () () () ()\n^entry():\nx x\nperm_action @expected (x:wire mut) () () () ()\n^entry():\nx x\n").materialize().unwrap();
    let h = o.hash().unwrap().symbols["expected"]
        .closure_hash
        .clone()
        .unwrap();
    bennett_circuits::certificate::generate_program(
        &o,
        "a",
        &[],
        None,
        true,
        &context(&target()),
        &h,
    )
    .unwrap();
}
#[test]
fn interface_only_symbolic_loops_are_checked_by_composition_not_unrolled() {
    let o = object(
        "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (t:wire mut, fr:wire[4] mut) (x:wire in) () (tk:token) ()\nregs {  } bounds {1000000000000}\n^entry():\nqloop 1000000000000 {\noracle_call @o tk (x) t fr () ()\n}\n",
    );
    o.hash().unwrap();
}
#[test]
fn budget_guarded_qinplace_does_not_get_a_parameter_uniform_adjoint_claim() {
    use bennett_circuits::oracle::{Argument, Budget};
    let o=object("oracle @g (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\noracle @gi (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (x:wire mut) () () (tk:token) ()\n^entry():\nqinplace @g @gi tk x\n").materialize().unwrap();
    let args = [Argument::Token(Budget {
        steps: 1,
        heap: 0,
        stack: 0,
        depth: 0,
    })];
    let cert =
        bennett_circuits::certificate::generate(&o, "a", &args, None, true, &context(&target()))
            .unwrap();
    let claims = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .get("claims")
        .unwrap()
        .as_array()
        .unwrap();
    let adj = claims
        .iter()
        .find(|c| c.get("rule").and_then(Value::as_str) == Some("adjoint_table"))
        .unwrap();
    assert_eq!(adj.get("invocation"), claims[0].get("invocation"));
}
#[test]
fn nested_qinplace_never_emits_a_packet_missing_its_owner_proof() {
    use bennett_circuits::oracle::{Argument, Budget};
    let o=object("oracle @g (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\noracle @gi (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @helper (x:wire mut) () () (tk:token) ()\n^entry():\nqinplace @g @gi tk x\naction @root (x:wire mut) () () (tk:token) ()\n^entry():\naction_call @helper (x) () () (tk)\n").materialize().unwrap();
    let args = [Argument::Token(Budget {
        steps: 1,
        heap: 0,
        stack: 0,
        depth: 0,
    })];
    assert!(
        bennett_circuits::certificate::generate(&o, "root", &args, None, true, &context(&target()))
            .unwrap_err()
            .0
            .contains("bennett-circuits-4pf")
    );
}
#[test]
fn draft_certificate_cannot_relabel_the_bound_source_isa_or_abi() {
    let mut o = object("perm_action @a (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    let cert =
        bennett_circuits::certificate::generate(&o, "a", &[], None, true, &context(&target()))
            .unwrap();
    o.draft_encodings = true;
    for key in ["isa", "object", "launch", "sched", "coverage", "target"] {
        let bad = cert
            .to_jcs()
            .replace(&format!("\"{key}\":\"0.8\""), &format!("\"{key}\":\"0.9\""));
        o.certificate = Some(Value::parse(&bad).unwrap());
        assert!(o.check().is_err(), "accepted conflicting {key} version");
    }
}
#[test]
fn oracle_fault_histograms_are_computed_over_the_declared_full_domain() {
    use bennett_circuits::oracle::Budget;
    let o = object("oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\n")
        .materialize()
        .unwrap();
    let cert = bennett_circuits::certificate::generate(
        &o,
        "o",
        &[],
        Some(Budget {
            steps: 0,
            heap: 0,
            stack: 0,
            depth: 0,
        }),
        true,
        &context(&target()),
    )
    .unwrap();
    let region = cert
        .get("regions")
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(
        region
            .get("verification")
            .unwrap()
            .get("fault_rate")
            .unwrap()
            .get("faults_by_code")
            .unwrap()
            .get("1")
            .unwrap()
            .as_u64(),
        Some(64)
    );
}
#[test]
fn oracle_reference_program_hash_is_checked_before_claim_replay() {
    use bennett_circuits::oracle::Budget;
    let mut o =
        object("oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\n")
            .materialize()
            .unwrap();
    let cert = bennett_circuits::certificate::generate(
        &o,
        "o",
        &[],
        Some(Budget {
            steps: 1,
            heap: 0,
            stack: 0,
            depth: 0,
        }),
        true,
        &context(&target()),
    )
    .unwrap();
    let h = cert
        .get("bindings")
        .unwrap()
        .get("reference_program_hash")
        .unwrap()
        .as_str()
        .unwrap();
    let bad = cert.to_jcs().replace(
        &format!("\"reference_program_hash\":\"{h}\""),
        &format!("\"reference_program_hash\":\"{}\"", "0".repeat(64)),
    );
    o.draft_encodings = true;
    o.certificate = Some(Value::parse(&bad).unwrap());
    assert!(o.check().is_err());
}
#[test]
fn linking_preserves_a_certificate_subject_instead_of_guessing_an_entry() {
    let mut leaf = object("perm_action @leaf (x:wire mut) () () () ()\n^entry():\nx x\n")
        .materialize()
        .unwrap();
    let hash = leaf.hash().unwrap().symbols["leaf"]
        .closure_hash
        .clone()
        .unwrap();
    leaf.draft_encodings = true;
    leaf.certificate = Some(
        bennett_circuits::certificate::generate(
            &leaf,
            "leaf",
            &[],
            None,
            true,
            &context(&target()),
        )
        .unwrap(),
    );
    let mut root = object(
        "perm_action @root (x:wire mut) () () () ()\n^entry():\naction_call @leaf (x) () () ()\n",
    );
    root.imports.push(bennett_circuits::object::Import {
        name: "leaf".into(),
        signature: leaf.symbols[0].signature.clone(),
        pinned_closure_hash: None,
    });
    let linked = link(&root, &[leaf]).unwrap();
    assert_eq!(
        linked
            .object
            .certificate
            .as_ref()
            .unwrap()
            .get("bindings")
            .unwrap()
            .get("closure_hash")
            .unwrap()
            .as_str(),
        Some(hash.as_str())
    );
}
