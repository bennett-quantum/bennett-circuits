//! Concrete invocation bindings for the discovered reference fixtures.
//! These are execution witnesses, never parameter-uniform cleanup proofs.
use bennett_circuits::{
    object::{Function, Type},
    oracle::{Argument, Budget},
};
pub fn arguments(f: &Function) -> Vec<Argument> {
    f.groups[3]
        .iter()
        .map(|p| match p.ty {
            Type::Token => Argument::Token(Budget {
                steps: 1,
                heap: 0,
                stack: 0,
                depth: 0,
            }),
            _ => panic!(
                "fixture @{} needs an explicit witness for {}",
                f.name, p.name
            ),
        })
        .collect()
}
pub fn cli(args: &[Argument]) -> Vec<String> {
    args.iter()
        .flat_map(|a| {
            let value = match a {
                Argument::Token(t) => {
                    format!("token:{},{},{},{}", t.steps, t.heap, t.stack, t.depth)
                }
                _ => panic!("fixture CLI binding is not a token"),
            };
            ["--arg".into(), value]
        })
        .collect()
}
