use bennett_circuits::{
    classical,
    object::{Object, Symbol, derived_signature},
};

fn object(body: &str) -> Object {
    let mut o = Object::parse(&format!("bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\n{body}")).unwrap();
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
    let o = o.materialize().unwrap();
    o.check().unwrap();
    o
}

#[test]
fn signed_const_must_execute_modulo_width() {
    for tier in ["cc", "r"] {
        for (width, literal, expected) in [
            (1, "-1", 1),
            (8, "-1", 255),
            (8, "-128", 128),
            (8, "-129", 127),
            (8, "-256", 0),
            (8, "256", 0),
            (32, "-1", u32::MAX as u128),
            (64, "-1", u64::MAX as u128),
            (128, "-1", u128::MAX),
            (128, "-340282366920938463463374607431768211456", 0),
            (
                128,
                "-170141183460469231731687303715884105728",
                1u128 << 127,
            ),
        ] {
            let o = object(&format!(
                "fn {tier} @a () (r:i{width}) () () ()\n^entry():\nr = const.i{width} {literal}\nret r\n"
            ));
            for reverse in [false, true] {
                let result = classical::run(&o, "a", &[], reverse).unwrap();
                assert_eq!(
                    result.get("status").unwrap().as_str(),
                    Some(if reverse { "REVERSED" } else { "COMPLETED" })
                );
                let value = &result.get("results").unwrap().as_array().unwrap()[0];
                assert_eq!(
                    value.get("value").unwrap().as_str(),
                    Some(expected.to_string().as_str())
                );
            }
        }
    }
}

#[test]
fn cc_region_call_must_forward_classical_arguments() {
    let body = "perm_action @flip (t:wire mut) () () (k:i8) ()\n^entry():\nx t\nfn cc @a () (r:i1) () () ()\n^entry():\nn = const.i8 1\nq = qreg.new any 1\naction_call @flip (q) () () (n)\nr = measure q\nret r\n";
    for adjoint in [false, true] {
        let body = if adjoint {
            body.replace(
                "r = measure",
                "adj action_call @flip (q) () () (n)\nr = measure",
            )
        } else {
            body.into()
        };
        let result = classical::run(&object(&body), "a", &[], false).unwrap();
        assert_eq!(result.get("status").unwrap().as_str(), Some("COMPLETED"));
        assert_eq!(
            result.get("results").unwrap().as_array().unwrap()[0]
                .get("value")
                .unwrap()
                .as_str(),
            Some(if adjoint { "0" } else { "1" })
        );
    }
}

#[test]
fn r_region_call_forwards_integer_arguments_from_cc_caller() {
    for adjoint in [false, true] {
        let inverse = if adjoint {
            "adj action_call @flip (q) () () (n)\n"
        } else {
            ""
        };
        let o = object(&format!(
            "perm_action @flip (t:wire mut) () () (k:i8) ()\n^entry():\nx t\nfn r @invoke (q:qreg[1], n:i8) () () () ()\n^entry():\naction_call @flip (q) () () (n)\n{inverse}ret\nfn cc @a () (r:i1) () () ()\n^entry():\nn = const.i8 255\nq = qreg.new any 1\ncall @invoke q n\nr = measure q\nret r\n"
        ));
        let result = classical::run(&o, "a", &[], false).unwrap();
        assert_eq!(result.get("status").unwrap().as_str(), Some("COMPLETED"));
        assert_eq!(
            result.get("results").unwrap().as_array().unwrap()[0]
                .get("value")
                .unwrap()
                .as_str(),
            Some(if adjoint { "0" } else { "1" })
        );
    }
}
