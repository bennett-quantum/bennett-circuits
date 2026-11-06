use bennett_circuits::{
    classical,
    object::{Object, Symbol, derived_signature},
};

fn materialized_object(body: &str) -> Object {
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
    o.materialize().unwrap()
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
            let o = materialized_object(&format!(
                "fn {tier} @a () (r:i{width}) () () ()\n^entry():\nr = const.i{width} {literal}\nret r\n"
            ));
            o.check().unwrap();
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

// ISA 0.8 §§3.3, 4.6, 5.3: with little-endian wires, x t[0]; cx t[0] t[1]
// cycles 0 -> 3 -> 2 -> 1 -> 0. With n=1, m=3, the final x t[1] loop
// gives 0 -> 3 -> 1. The adjoint starts at 0 independently: 0 -> 2 -> 3.
// Exercise argument binding directly: whole-object checking does not yet
// support symbolic trip counts, which lowering resolves at each region call.
const COUNTED_ACTION: &str = "\
perm_action @counted (t:wire[2] mut) () () (n:i8, m:i8) ()
regs { } bounds {1, 3}
^entry():
qloop n {
x t[0]
cx t[0] t[1]
}
qloop m {
x t[1]
}
";

#[test]
fn cc_region_call_must_forward_classical_arguments() {
    for adjoint in [false, true] {
        let adj = if adjoint { "adj " } else { "" };
        let body = format!(
            "{COUNTED_ACTION}fn cc @a () (r:i2) () () ()\n^entry():\nn = const.i8 1\nm = const.i8 3\nq = qreg.new any 2\n{adj}action_call @counted (q) () () (n, m)\nr = measure q\nret r\n"
        );
        let result = classical::run(&materialized_object(&body), "a", &[], false).unwrap();
        assert_eq!(result.get("status").unwrap().as_str(), Some("COMPLETED"));
        assert_eq!(
            result.get("results").unwrap().as_array().unwrap()[0]
                .get("value")
                .unwrap()
                .as_str(),
            Some(if adjoint { "3" } else { "1" })
        );
    }
}

#[test]
fn r_region_call_forwards_integer_arguments_from_cc_caller() {
    for adjoint in [false, true] {
        let adj = if adjoint { "adj " } else { "" };
        let o = materialized_object(&format!(
            "{COUNTED_ACTION}fn r @invoke (q:qreg[2], n:i8, m:i8) () () () ()\n^entry():\n{adj}action_call @counted (q) () () (n, m)\nret\nfn cc @a () (r:i2) () () ()\n^entry():\nn = const.i8 1\nm = const.i8 3\nq = qreg.new any 2\ncall @invoke q n m\nr = measure q\nret r\n"
        ));
        let result = classical::run(&o, "a", &[], false).unwrap();
        assert_eq!(result.get("status").unwrap().as_str(), Some("COMPLETED"));
        assert_eq!(
            result.get("results").unwrap().as_array().unwrap()[0]
                .get("value")
                .unwrap()
                .as_str(),
            Some(if adjoint { "3" } else { "1" })
        );
    }
}
