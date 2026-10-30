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
fn matched_cleanup_tracks_indirect_labels_with_unrelated_lprocs() {
    let body = "lift.oracle_call @inc8 tk (x) r fr () ()\nidx = lift.label ^cleanup\nnext = lift.mov idx\nqjumpr next\n^cleanup():\nlift.oracle_uncall @inc8 tk (x) r fr () ()\nlift.retval r";
    let mut o = nested_object(body);
    let f = o.functions.iter_mut().find(|f| f.name == "cancel").unwrap();
    for name in ["idx", "next"] {
        f.regs.push(bennett_circuits::object::RegDecl {
            name: name.into(),
            ty: bennett_circuits::object::Type::Reg(32),
        });
    }
    let extra = generated("lproc @unused () (r:reg[8]) () () ()\n^entry():\nlift.ret r\n");
    o.functions.extend(extra.functions);
    let mut s = extra.symbols[0].clone();
    s.index = o.symbols.len().to_string();
    o.symbols.push(s);
    let memory = Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/sum_db.bisa").unwrap()).unwrap();
    o.layout = memory.layout;
    if let Some(Value::Object(layout)) = &mut o.layout {
        layout.insert("globals".into(), Value::Object(Default::default()));
        layout.insert(
            "interface_regions".into(),
            Value::Object(Default::default()),
        );
    }
    o.hash().unwrap();
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

#[test]
fn nested_metadata_schemas_reject_unknown_fields_and_wrong_types() {
    let root = "/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects";
    for (file, before, after) in [
        (
            "f_import_unlinked",
            "\"tgt\":8",
            "\"tgt\":8,\"unexpected\":0",
        ),
        ("f_import_unlinked", "\"fault\":4", "\"fault\":\"wrong\""),
        (
            "classical",
            "\"clock\":{\"version\":\"0.8\"}",
            "\"clock\":{\"extra\":0,\"version\":\"0.8\"}",
        ),
        ("classical", "\"globals\":{}", "\"globals\":[]"),
        (
            "sched_inc8",
            "\"start\":{\"time\":0}",
            "\"start\":{\"extra\":0,\"time\":0}",
        ),
        ("sched_inc8", "\"iteration\":[]", "\"iteration\":false"),
    ] {
        let text = std::fs::read_to_string(format!("{root}/{file}.bisa")).unwrap();
        let text = if file == "f_import_unlinked" {
            let (prefix, rest) = text.split_once("section imports\n").unwrap();
            format!("{prefix}section imports\n{}", rest.replace(before, after))
        } else {
            text.replace(before, after)
        };
        let text = text
            .lines()
            .map(|l| {
                if l.starts_with('{') {
                    Value::parse(l).unwrap().to_jcs()
                } else {
                    l.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let e = Object::parse(&text).and_then(|o| o.hash()).expect_err(file);
        assert!(e.0.contains("§9.1"), "{file}: {e}");
    }
}

fn generated(body: &str) -> Object {
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
    o
}
#[test]
fn handle_liveness_is_path_sensitive_and_renames_block_arguments() {
    for body in [
        "fn cc @a (p:i1) () () () ()\n^entry():\nq = qreg.new any 1\nbr p ^left() ^right()\n^left():\nqreg.free q\nbr ^done()\n^right():\nqreg.free q\nbr ^done()\n^done():\nret\n",
        "fn cc @a () () () () ()\n^entry():\nq = qreg.new any 1\nbr ^done(q)\n^done(r:qreg[1]):\nqreg.free r\nret\n",
        "fn cc @a (p:i1) () () () ()\n^entry():\nbr ^loop()\n^loop():\nq = qreg.new any 1\nqreg.free q\nbr p ^loop() ^done()\n^done():\nret\n",
    ] {
        generated(body).hash().unwrap();
    }
}
#[test]
fn handle_join_rejects_consumed_path_and_view_escape() {
    for body in [
        "fn cc @a (p:i1) () () () ()\n^entry():\nq = qreg.new any 1\nbr p ^left() ^done()\n^left():\nqreg.free q\nbr ^done()\n^done():\nqreg.free q\nret\n",
        "fn cc @a () () () () ()\n^entry():\nq = qreg.new any 2\nv = qreg.slice q 0 1\nbr ^done()\n^done():\nret\n",
    ] {
        let e = generated(body).hash().unwrap_err();
        assert!(
            e.0.contains("§3.3") && !e.0.contains("unimplemented"),
            "{e}"
        );
    }
}

fn with_borrow_certificate(o: &mut Object, sampled: bool, truncate_domain: bool) {
    // Use an independent no-borrow copy to obtain the semantic identity;
    // identity excludes certificate metadata (§9.2).
    let mut identity = o.clone();
    identity.functions[0].blocks[0].instrs.clear();
    let hashes = identity.hash().unwrap();
    // Function canonical bytes, used by identity, were not changed.
    let h = hashes.symbols["a"].closure_hash.as_ref().unwrap();
    let kind = if sampled { "sampled" } else { "exhaustive" };
    let hi = if truncate_domain { "0" } else { "1" };
    o.certificate=Some(Value::parse(&format!(r#"{{"bindings":{{"closure_hash":"{h}","interface_hash":"{h}","object_hash":"{}"}},"composition":{{"epsilon_total":{{"diamond":0}},"method":"sum_over_multiplicities","multiplicities":{{}}}},"producer":{{"materialisation":{{"llvm_version":"none","normaliser_version":"1","pipeline":[]}},"name":"test","version":"1"}},"regions":{{"{h}":{{"claims":[{{"domain":{{"borrowed":[{{"hi":"{hi}","lo":"0","name":"b","width":1}}],"bundles":[{{"hi":"1","lo":"0","name":"t","width":1}}],"regions":[]}},"expected":{{"program":"{h}"}},"kind":"{kind}"}}],"verification":{{"borrow_frames":[0]}}}}}},"versions":{{"cert":"0.8"}}}}"#,o.object_hash())).unwrap());
    if sampled {
        let Value::Object(cert) = o.certificate.as_mut().unwrap() else {
            panic!()
        };
        let Value::Object(regions) = cert.get_mut("regions").unwrap() else {
            panic!()
        };
        let Value::Object(region) = regions.get_mut(h).unwrap() else {
            panic!()
        };
        let Value::Array(claims) = region.get_mut("claims").unwrap() else {
            panic!()
        };
        let Value::Object(claim) = &mut claims[0] else {
            panic!()
        };
        claim.insert(
            "sampler".into(),
            Value::parse(r#"{"algorithm":"xoshiro256**","count":1,"seed":["1","0","0","0"]}"#)
                .unwrap(),
        );
    }
}
#[test]
fn exhaustive_borrow_certificate_checks_full_domain_and_factorisation() {
    let body = "perm_action @a (t:wire mut) () () () ()\n^entry():\nb = qborrow any 1\nswap b t\nswap b t\nqreturn b\n";
    let mut o = generated(body);
    with_borrow_certificate(&mut o, false, false);
    o.hash().unwrap();
    for (sampled, truncated) in [(true, false), (false, true)] {
        let mut o = generated(body);
        with_borrow_certificate(&mut o, sampled, truncated);
        let e = o.hash().unwrap_err();
        assert!(e.0.contains("§5.6"), "{e}");
    }
    let mut o = generated(&body.replace("swap b t\nswap b t", "cx b t"));
    with_borrow_certificate(&mut o, false, false);
    let e = o.hash().unwrap_err();
    assert!(e.0.contains("§5.6"), "{e}");
}
#[test]
fn borrow_certificate_cannot_rebind_to_another_object() {
    let mut o = generated(
        "perm_action @a (t:wire mut) () () () ()\n^entry():\nb = qborrow any 1\nswap b t\nswap b t\nqreturn b\n",
    );
    with_borrow_certificate(&mut o, false, false);
    let Value::Object(cert) = o.certificate.as_mut().unwrap() else {
        panic!()
    };
    let Value::Object(bindings) = cert.get_mut("bindings").unwrap() else {
        panic!()
    };
    bindings.insert("object_hash".into(), Value::Str("0".repeat(64)));
    let e = o.hash().unwrap_err();
    assert!(e.0.contains("§12"), "{e}");
}

#[test]
fn quantum_cleanup_cannot_erase_interference_or_entanglement() {
    for gates in ["h d\ns d\ns d\nh d", "h d\ncx d t\nh d"] {
        let o = generated(&format!(
            "action @a (t:wire mut) () () () ()\n^entry():\nd = qalloc any 1\n{gates}\nqfree d\n"
        ));
        let e = o.hash().unwrap_err();
        assert!(e.0.contains("§6.2"), "{e}");
    }
    for gates in ["h d\ns d\nsdg d\nh d", "h d\ncx d t\ncx d t\nh d"] {
        generated(&format!(
            "action @a (t:wire mut) () () () ()\n^entry():\nd = qalloc any 1\n{gates}\nqfree d\n"
        ))
        .hash()
        .unwrap();
    }
}

#[test]
fn certificate_nested_fields_are_validated_before_claim_use() {
    let mut o = generated(
        "perm_action @a (t:wire mut) () () () ()\n^entry():\nb = qborrow any 1\nswap b t\nswap b t\nqreturn b\n",
    );
    with_borrow_certificate(&mut o, false, false);
    let Value::Object(cert) = o.certificate.as_mut().unwrap() else {
        panic!()
    };
    let Value::Object(producer) = cert.get_mut("producer").unwrap() else {
        panic!()
    };
    producer.insert("unknown".into(), Value::Int("0".into()));
    let e = o.hash().unwrap_err();
    assert!(e.0.contains("§9.1"), "{e}");
}

#[test]
fn unreachable_classical_blocks_still_require_valid_row_schemas() {
    let o = generated("fn cc @a () () () () ()\n^entry():\nret\n^dead():\nbogus\nret\n");
    let e = o.hash().unwrap_err();
    assert!(e.0.contains("classical schema bogus"), "{e}");
}

#[test]
fn selectors_and_while_dependencies_cannot_be_modified_even_if_undone() {
    for body in [
        "perm_action @a (c:wire mut) () () () ()\nregs {  } bounds {1}\n^entry():\nqloop 1 while c {\nx c\nx c\n}\n",
        "perm_action @a (p:wire mut) () () () ()\nregs {  } bounds {1}\n^entry():\nc = qcopy p\nqloop 1 while c {\nx p\nx p\n}\nquncopy p c\n",
    ] {
        let e = generated(body).hash().unwrap_err();
        assert!(e.0.contains("§3.3"), "{e}");
    }
    generated("perm_action @a (t:wire[2] mut) () () () ()\n^entry():\nqif t[0] {\nx t[1]\n}\n")
        .hash()
        .unwrap();
}

fn materialise(body: &str, label: &str) -> std::path::PathBuf {
    use bennett_circuits::sha256::sha256_hex;
    let o = generated(body);
    let mut symbols = std::collections::BTreeMap::new();
    for symbol in &o.symbols {
        let h = sha256_hex(&o.semantic_envelope(&symbol.name).unwrap());
        let entry = Value::parse(&format!(
            r#"{{"interface_hash":"{h}","kind":"{}","name":"{}","signature":{}}}"#,
            symbol.kind,
            symbol.name,
            symbol.signature.to_jcs()
        ))
        .unwrap();
        symbols.insert(symbol.index.clone(), entry);
    }
    let text = format!(
        "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{}\n\nsection functions\n{body}",
        Value::Object(symbols).to_jcs()
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/checker-witnesses");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("{label}.bisa"));
    std::fs::write(&path, text).unwrap();
    path
}
#[test]
fn new_cleanup_and_selector_witnesses_agree_with_reference() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for (label, body) in [
        (
            "interference",
            "action @a (t:wire mut) () () () ()\n^entry():\nd = qalloc any 1\nh d\ns d\ns d\nh d\nqfree d\n",
        ),
        (
            "entanglement",
            "action @a (t:wire mut) () () () ()\n^entry():\nd = qalloc any 1\nh d\ncx d t\nh d\nqfree d\n",
        ),
        (
            "selector",
            "perm_action @a (c:wire mut) () () () ()\nregs {  } bounds {1}\n^entry():\nqloop 1 while c {\nx c\nx c\n}\n",
        ),
        (
            "nonperm-call-window",
            "action @id (x:wire mut) () () () ()\n^entry():\ns x\ns x\ns x\ns x\naction @a (x:wire mut) () () () ()\n^entry():\nd = qcopy x\naction_call @id (x) () () ()\nquncopy x d\n",
        ),
        (
            "indirect-matched-cleanup",
            "oracle @id (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x\noracle @a (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1], fr:reg[4], idx:reg[32], next:reg[32] }\n^entry():\nlift.oracle_call @id tk (x) r fr () ()\nidx = lift.label ^cleanup\nnext = lift.mov idx\nqjumpr next\n^cleanup():\nlift.oracle_uncall @id tk (x) r fr () ()\nlift.retval r\n",
        ),
        (
            "unused-token-workspace",
            "action @a (t:wire mut) () () (tk:token) ()\n^entry():\nd = qcopy t\ns d\nquncopy t d\n",
        ),
        (
            "unused-token-window",
            "action @a (t:wire mut) () () (tk:token) ()\n^entry():\nd = qcopy t\nh t\nquncopy t d\n",
        ),
        (
            "symbolic-call-workspace-sections",
            "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (t:wire mut, fr:wire[4] mut) (x:wire in) () (tk:token) ()\n^entry():\nd = qcopy x\ns d\nquncopy x d\noracle_call @o tk (x) t fr () ()\ne = qcopy t\ns e\nquncopy t e\n",
        ),
        (
            "symbolic-count-clean-workspace",
            "action @a (t:wire mut) () () (n:i8) ()\nregs {  } bounds {3}\n^entry():\nqloop n {\nd = qcopy t\ns d\nquncopy t d\n}\n",
        ),
    ] {
        let path = materialise(body, label);
        let ours = Object::parse(&std::fs::read_to_string(&path).unwrap()).and_then(|o| o.check());
        let reference = Command::new(
            base.join("bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref"),
        )
        .arg("check")
        .arg(&path)
        .output()
        .unwrap();
        assert_eq!(
            ours.is_ok(),
            reference.status.success(),
            "{label}: ours={ours:?}; reference stdout={} stderr={}",
            String::from_utf8_lossy(&reference.stdout),
            String::from_utf8_lossy(&reference.stderr)
        );
        if let Err(e) = &ours {
            let diagnostic = String::from_utf8_lossy(&reference.stderr);
            let rule = diagnostic
                .split_once('§')
                .expect("reference rule citation")
                .1
                .split(|c: char| !c.is_ascii_digit() && c != '.')
                .next()
                .unwrap()
                .trim_end_matches('.');
            assert!(
                e.0.contains(&format!("§{rule}")),
                "{label}: ours={e}; reference={diagnostic}"
            );
        }
    }
}

#[test]
fn imported_lproc_and_classical_calls_use_published_signature_schemas() {
    use bennett_circuits::object::Import;
    let text=std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/sum_db.bisa").unwrap();
    let mut o = Object::parse(&text).unwrap();
    let sym = o
        .symbols
        .remove(o.symbols.iter().position(|s| s.name == "sum").unwrap());
    o.functions.retain(|f| f.name != "sum");
    o.imports.push(Import {
        name: "sum".into(),
        signature: sym.signature,
        pinned_closure_hash: None,
    });
    assert_eq!(o.hash().unwrap().status, "unlinked");
    let mut o =
        generated("fn cc @a (x:i8) (r:i8) () () ()\n^entry():\ny = call @external x\nret y\n");
    o.imports.push(Import {
        name: "external".into(),
        signature: Value::parse(
            r#"{"abi":"0.8","kind":"r","params":["i8"],"results":["i8"],"tier":"r"}"#,
        )
        .unwrap(),
        pinned_closure_hash: None,
    });
    assert_eq!(o.hash().unwrap().status, "unlinked");
}

#[test]
fn imported_region_contracts_bind_cc_handles_without_a_body() {
    use bennett_circuits::object::Import;
    for (body, sig) in [
        (
            "fn cc @a (q:qreg[1]) () () () ()\n^entry():\naction_call @external (q) () () ()\nret\n",
            r#"{"abi":"0.8","kind":"action","tier":"q","mut":[1],"in":[],"mem":[],"args":[],"bounds":[]}"#,
        ),
        (
            "fn cc @a (q:qreg[1], x:qreg[1], fr:qreg[4], tk:token) () () () ()\n^entry():\noracle_call @external tk (x) q fr () ()\nret\n",
            r#"{"abi":"0.8","kind":"oracle","tier":"q","in":[1],"tgt":1,"fault":4,"mem":[],"args":[]}"#,
        ),
    ] {
        let mut o = generated(body);
        o.imports.push(Import {
            name: "external".into(),
            signature: Value::parse(sig).unwrap(),
            pinned_closure_hash: None,
        });
        assert_eq!(o.hash().unwrap().status, "unlinked");
    }
}

#[test]
fn nonpermutation_call_closes_dependency_window_even_for_identity_body() {
    let body = "action @id (x:wire mut) () () () ()\n^entry():\ns x\ns x\ns x\ns x\naction @a (x:wire mut) () () () ()\n^entry():\nd = qcopy x\naction_call @id (x) () () ()\nquncopy x d\n";
    let e = generated(body)
        .hash()
        .expect_err("opaque non-permutation call must close the window");
    assert!(e.0.contains("§6.2"), "{e}");
}

#[test]
fn symbolic_angles_validate_exact_adjoint_histories_for_all_arguments() {
    let prefix = "action @rot (x:wire mut) () () (theta:f64) ()\n^entry():\nrx theta x\n";
    let good = format!(
        "{prefix}action @a (t:wire mut) () () (alpha:f64, beta:f64) ()\n^entry():\nd = qalloc any 1\naction_call @rot (d) () () (alpha)\nadj action_call @rot (d) () () (alpha)\nqfree d\n"
    );
    generated(&good).hash().unwrap();
    for bad in [
        good.replace(
            "adj action_call @rot (d) () () (alpha)",
            "adj action_call @rot (d) () () (2)",
        ),
        good.replace(
            "adj action_call @rot (d) () () (alpha)",
            "adj action_call @rot (d) () () (beta)",
        ),
        good.replace("qfree d", "s t\nqfree d")
            .replace("adj action_call @rot (d) () () (alpha)\n", ""),
    ] {
        let e = generated(&bad).hash().unwrap_err();
        assert!(
            e.0.contains("§6.2") && !e.0.contains("unimplemented"),
            "{e}"
        );
    }
}

#[test]
fn translation_lemma_is_checked_per_maximal_permutation_frame() {
    let frame = "cx x b\ncx b y\ncx x b\ncx b y\n";
    let body = format!(
        "action @a (y:wire mut, t:wire mut) (x:wire in) () () ()\n^entry():\nb = qborrow any 1\n{frame}h t\n{frame}qreturn b\n"
    );
    generated(&body).hash().unwrap();
    let path = materialise(&body, "separated-borrow-frames");
    let reference = Command::new("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref")
        .arg("check").arg(path).output().unwrap();
    assert!(
        reference.status.success(),
        "reference disagrees with §5.6 maximal contiguous permutation-only frames: {}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let bad = body.replace("h t", "h b");
    let e = generated(&bad).hash().unwrap_err();
    assert!(e.0.contains("§5.6"), "{e}");
}

#[test]
fn object_metadata_requires_wide_integers_to_use_decimal_strings() {
    let text=std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects/sched_inc8.bisa").unwrap();
    let (prefix, line) = text.split_once("section schedule\n").unwrap();
    let mut schedule = Value::parse(line.trim_end()).unwrap();
    let Value::Object(root) = &mut schedule else {
        unreachable!()
    };
    let Value::Object(binding) = root.get_mut("bindings").unwrap() else {
        unreachable!()
    };
    let Value::Array(budget) = binding.get_mut("budget").unwrap() else {
        unreachable!()
    };
    budget[0] = Value::Int("9007199254740993".into());
    let text = format!("{prefix}section schedule\n{}\n", schedule.to_jcs());
    let e = Object::parse(&text).and_then(|o| o.check()).unwrap_err();
    assert!(e.0.contains("§16.1"), "{e}");
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/unsafe-meta-int.bisa");
    std::fs::write(&p, text).unwrap();
    let reference=Command::new("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref").arg("check").arg(p).output().unwrap();
    assert!(!reference.status.success());
    assert!(String::from_utf8_lossy(&reference.stderr).contains("§16.1"));
}

#[test]
fn imported_action_cannot_write_a_readonly_caller_region() {
    use bennett_circuits::object::Import;
    let mut o = generated(
        "action @a () () (r:region[0,1] in) () ()\n^entry():\naction_call @external () () (r) ()\n",
    );
    o.imports.push(Import{name:"external".into(),pinned_closure_hash:None,signature:Value::parse(r#"{"abi":"0.8","kind":"action","tier":"q","mut":[],"in":[],"mem":[{"A":0,"W":1,"role":"mut"}],"args":[],"bounds":[]}"#).unwrap()});
    let e = o.hash().unwrap_err();
    assert!(
        e.0.contains("§5.1") && !e.0.contains("unimplemented"),
        "{e}"
    );
}

#[test]
fn inherited_control_schedule_audit_fails_explicitly_until_implemented() {
    let mut o = generated("action @a (t:wire mut) () () () ()\n^entry():\nx t\n");
    let source=Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects/sched_inc8.bisa").unwrap()).unwrap();
    let mut schedule = source.schedule.unwrap();
    let Value::Object(root) = &mut schedule else {
        unreachable!()
    };
    let Value::Object(b) = root.get_mut("bindings").unwrap() else {
        unreachable!()
    };
    b.insert("region".into(), Value::Str("a".into()));
    b.insert("variant".into(), Value::Str("controlled".into()));
    b.insert("inherited_controls".into(), Value::Int("1".into()));
    b.insert(
        "closure_hash".into(),
        Value::Str(o.hash().unwrap().symbols["a"].closure_hash.clone().unwrap()),
    );
    o.schedule = Some(schedule);
    let e = o.hash().unwrap_err();
    assert!(
        e.0.contains("unimplemented: §6.4") && e.0.contains("milestone 2"),
        "{e}"
    );
}

#[test]
fn parameterized_action_oracle_calls_bind_the_action_memory_group() {
    let mut o=Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/tiny_memory.bisa").unwrap()).unwrap();
    let mut action = generated(
        "perm_action @a (t:wire mut, fr:wire[4] mut) (x:wire in) (db:region[0,8] in) (tk:token) ()\n^entry():\noracle_call @tiny tk (x) t fr (db) ()\n",
    );
    action.symbols[0].index = "1".into();
    o.functions.extend(action.functions);
    o.symbols.extend(action.symbols);
    o.hash().unwrap();
}

#[test]
fn symbolic_interface_calls_preserve_predicates_controls_and_literal_loop_contracts() {
    let oracle = "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\nregs { r:reg[1] }\n^entry():\nr = lift.xor x x\nlift.retval r\n";
    let body = format!(
        "{oracle}perm_action @a (t:wire mut, fr:wire[4] mut) (x:wire in, p:wire in, q:wire in) () (tk:token) ()\nregs {{  }} bounds {{2}}\n^entry():\nqpred p\nqloop 2 {{\nctl q adj oracle_call @o tk (x) t fr () ()\n}}\nqunpred p\n"
    );
    generated(&body).hash().unwrap();
    let bad = body.replace("ctl q adj", "ctl t adj");
    let e = generated(&bad).hash().unwrap_err();
    assert!(
        e.0.contains("§3.3") && !e.0.contains("unimplemented"),
        "{e}"
    );
}

#[test]
fn symbolic_workspace_is_proved_without_instantiating_unused_parameters() {
    for params in ["n:i128", "tk:token", "n:i128, tk:token, theta:f64"] {
        let body = format!(
            "action @a (t:wire mut) () () ({params}) ()\n^entry():\nd = qcopy t\nz d\nquncopy t d\n"
        )
        .replace("z d", "s d");
        generated(&body).hash().unwrap();
        let bad = body.replace("s d", "h t");
        let e = generated(&bad).hash().unwrap_err();
        assert!(
            e.0.contains("§6.2") && !e.0.contains("unimplemented"),
            "{e}"
        );
    }
    generated("action @a (t:wire mut) () () (tk:token, theta:f64) ()\n^entry():\nd = qalloc any 1\nh d\nh d\nqfree d\n").hash().unwrap();
}

#[test]
fn serialized_symbolic_counts_keep_an_explicit_cited_refusal() {
    let body = "action @a (t:wire mut) () () (n:i8) ()\nregs {  } bounds {3}\n^entry():\nqloop n {\nd = qcopy t\ns d\nquncopy t d\n}\n";
    let e = generated(body).hash().unwrap_err();
    assert!(
        e.0.contains("unimplemented: §16.1") && e.0.contains("milestone 2"),
        "{e}"
    );
}

#[test]
fn symbolic_interface_calls_compose_with_independent_workspace_sections() {
    let prefix = "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\n";
    let body = format!(
        "{prefix}action @a (t:wire mut, fr:wire[4] mut) (x:wire in) () (tk:token) ()\n^entry():\nd = qcopy x\ns d\nquncopy x d\noracle_call @o tk (x) t fr () ()\ne = qcopy t\ns e\nquncopy t e\n"
    );
    generated(&body).hash().unwrap();
    let bad = body.replace("s e", "h t");
    let e = generated(&bad).hash().unwrap_err();
    assert!(
        e.0.contains("§6.2") && !e.0.contains("unimplemented"),
        "{e}"
    );
}

#[test]
fn symbolic_interface_calls_do_not_bypass_declared_loop_bounds() {
    let body = "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (t:wire mut, fr:wire[4] mut) (x:wire in) () (tk:token) ()\nregs {  } bounds {3}\n^entry():\nqloop 2 {\noracle_call @o tk (x) t fr () ()\n}\n";
    let e = generated(body).hash().unwrap_err();
    assert!(
        e.0.contains("§5.3") && !e.0.contains("unimplemented"),
        "{e}"
    );
}

#[test]
fn symbolic_workspace_spanning_an_opaque_call_stays_an_explicit_proof_gap() {
    let body = "oracle @o (x:wire in) (t:wire) (fr:wire[4]) () ()\n^entry():\nlift.retval x\naction @a (t:wire mut, fr:wire[4] mut) (x:wire in) () (tk:token) ()\n^entry():\nd = qcopy x\noracle_call @o tk (x) t fr () ()\nquncopy x d\n";
    let e = generated(body).hash().unwrap_err();
    assert!(
        e.0.contains("unimplemented: §6")
            && e.0.contains("milestone 2")
            && e.0.contains("bennett-circuits-7bu")
            && e.0.contains("bennett-circuits-z9r"),
        "{e}"
    );
}

#[test]
fn unused_symbolic_parameters_do_not_require_imported_interface_bodies() {
    use bennett_circuits::object::Import;
    let mut o = generated(
        "action @a (t:wire mut) () () (unused:token) ()\n^entry():\naction_call @external (t) () () ()\n",
    );
    o.imports.push(Import{name:"external".into(),pinned_closure_hash:None,signature:Value::parse(r#"{"abi":"0.8","kind":"action","tier":"q","mut":[1],"in":[],"mem":[],"args":[],"bounds":[]}"#).unwrap()});
    assert_eq!(o.hash().unwrap().status, "unlinked");
}

#[test]
fn symbolic_opaque_action_calls_use_the_full_interface_contract() {
    use bennett_circuits::object::Import;
    let mut o = generated(
        "action @a (t:wire mut) (p:wire in) () (n:i8) ()\n^entry():\nctl p action_call @external (t) () () (n)\n",
    );
    o.imports.push(Import{name:"external".into(),pinned_closure_hash:None,signature:Value::parse(r#"{"abi":"0.8","kind":"action","tier":"q","mut":[1],"in":[],"mem":[],"args":["i8"],"bounds":[]}"#).unwrap()});
    assert_eq!(o.hash().unwrap().status, "unlinked");
    o.imports[0].signature=Value::parse(r#"{"abi":"0.8","kind":"action","tier":"q","mut":[2],"in":[],"mem":[],"args":["i8"],"bounds":[]}"#).unwrap();
    let e = o.hash().unwrap_err();
    assert!(
        e.0.contains("§3.3") && !e.0.contains("unimplemented"),
        "{e}"
    );
}

#[test]
fn recompute_schedule_requires_an_implemented_eligibility_proof() {
    let mut o=Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/conformance/cases/objects/sched_inc8.bisa").unwrap()).unwrap();
    let Value::Object(schedule) = o.schedule.as_mut().unwrap() else {
        unreachable!()
    };
    let Value::Array(entries) = schedule.get_mut("entries").unwrap() else {
        unreachable!()
    };
    let Value::Object(entry) = &mut entries[0] else {
        unreachable!()
    };
    entry.remove("start");
    entry.insert("kind".into(), Value::Str("cleanup".into()));
    entry.insert(
        "cleanup".into(),
        Value::parse(r#"{"realisation":"recompute"}"#).unwrap(),
    );
    let e = o.hash().unwrap_err();
    assert!(
        e.0.contains("unimplemented: §6.4") && e.0.contains("milestone 2"),
        "{e}"
    );
    assert!(e.0.contains("b-isa-spec-8r4"), "{e}");
}

#[test]
fn inverse_library_borrow_claim_names_the_unpublished_contract_blocker() {
    let mut o = generated(
        "perm_action @a (t:wire mut) () () () ()\n^entry():\nb = qborrow any 1\nswap b t\nswap b t\nqreturn b\n",
    );
    with_borrow_certificate(&mut o, false, false);
    let Value::Object(cert) = o.certificate.as_mut().unwrap() else {
        panic!()
    };
    let Value::Object(regions) = cert.get_mut("regions").unwrap() else {
        panic!()
    };
    let Value::Object(region) = regions.values_mut().next().unwrap() else {
        panic!()
    };
    region.insert("claims".into(), Value::parse(&format!(r#"[{{"kind":"structural","rule":"inverse_pair_library","subclaims":[],"witness":{{"library_closure_hash":"{}","claim_index":0}}}}]"#, "0".repeat(64))).unwrap());
    let e = o.hash().unwrap_err();
    assert!(
        e.0.contains("unimplemented: §12")
            && e.0.contains("b-isa-spec-8r4")
            && e.0.contains("milestone 2"),
        "{e}"
    );
}

#[test]
fn overlapping_cyclic_invocations_match_multiplicity_and_current_versions() {
    let prefix = "oracle @o (x:wire in) (t:wire) (f:wire[4]) () ()\n^entry():\nlift.retval x\noracle @a (x:wire in) (t:wire) (f:wire[4]) () (tk:token)\nregs { r:reg[1], fr:reg[4] }\n^entry():\n";
    let call = "lift.oracle_call @o tk (x) r fr () ()\n";
    let uncall = "lift.oracle_uncall @o tk (x) r fr () ()\n";
    let cases = [
        (
            "cyclic-latest",
            format!(
                "qjump ^again\n^again():\n{call}qbranch x ^again ^cleanup\n^cleanup():\n{uncall}lift.retval r\n"
            ),
            true,
        ),
        (
            "cyclic-two-required",
            format!(
                "qjump ^again\n^again():\n{call}qbranch x ^again ^cleanup\n^cleanup():\n{uncall}{uncall}lift.retval r\n"
            ),
            false,
        ),
        (
            "two-retained",
            format!("{call}{call}{uncall}{uncall}lift.retval r\n"),
            true,
        ),
        (
            "three-required",
            format!("{call}{call}{uncall}{uncall}{uncall}lift.retval r\n"),
            false,
        ),
        (
            "delivery-overwritten",
            format!("{call}r = lift.mov x\n{uncall}lift.retval r\n"),
            false,
        ),
        (
            "retained-then-balanced-loop",
            format!(
                "{call}qjump ^again\n^again():\n{call}{uncall}qbranch x ^again ^cleanup\n^cleanup():\n{uncall}lift.retval r\n"
            ),
            true,
        ),
    ];
    for (label, tail, valid) in cases {
        let path = materialise(&format!("{prefix}{tail}"), label);
        let reference = Command::new("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref")
            .arg("check").arg(&path).output().unwrap();
        assert_eq!(
            reference.status.success(),
            valid,
            "{label}: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        let ours = Object::parse(&std::fs::read_to_string(path).unwrap()).and_then(|o| o.check());
        assert_eq!(ours.is_ok(), valid, "{label}: {ours:?}");
        if let Err(e) = ours {
            assert!(
                e.0.contains("§5.5") && !e.0.contains("unimplemented"),
                "{label}: {e}"
            );
        }
    }
    eprintln!("CYCLIC HARNESS: 6 check comparisons agree");
}

#[test]
fn undecided_frame_and_replay_contracts_name_their_beads() {
    for (field, value, bead) in [
        ("frame", r#"{}"#, "bennett-circuits-4fa"),
        (
            "claims",
            r#"[{"kind":"exhaustive","domain":{"bundles":[{"name":"t","width":1,"lo":"0","hi":"1"}],"borrowed":[{"name":"b","width":1,"lo":"0","hi":"1"}],"regions":[]},"expected":{"table":[]}}]"#,
            "bennett-circuits-4pf",
        ),
    ] {
        let mut o = generated(
            "perm_action @a (t:wire mut) () () () ()\n^entry():\nb = qborrow any 1\nswap b t\nswap b t\nqreturn b\n",
        );
        with_borrow_certificate(&mut o, false, false);
        let Value::Object(cert) = o.certificate.as_mut().unwrap() else {
            panic!()
        };
        let Value::Object(regions) = cert.get_mut("regions").unwrap() else {
            panic!()
        };
        let Value::Object(region) = regions.values_mut().next().unwrap() else {
            panic!()
        };
        region.insert(field.into(), Value::parse(value).unwrap());
        let e = o.hash().unwrap_err();
        assert!(e.0.contains("unimplemented:") && e.0.contains(bead), "{e}");
    }
}
