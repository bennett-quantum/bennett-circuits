#[path = "support/exact.rs"]
mod exact;
#[path = "support/sim.rs"]
mod sim;
use bennett_circuits::{
    flatten::flatten,
    object::Object,
    oracle::{Budget, specialise},
};
#[test]
fn literal_inc8_commits_and_reverses_all_workspace() {
    let o=Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/inc8.bisa").unwrap()).unwrap();
    for steps in [0, 2, 3, 4] {
        let p = specialise(
            &o,
            "inc8",
            Budget {
                steps,
                heap: 0,
                stack: 0,
                depth: 0,
            },
            &[],
        )
        .unwrap();
        let f = flatten(&p).unwrap();
        // Interface: tgt, fault, input. Arbitrary target/fault entries.
        for x in [0u16, 1, 127, 255] {
            for t in [0u16, 57, 255] {
                for c in [0u16, 7, 15] {
                    let input = (t as u32) | ((c as u32) << 8) | ((x as u32) << 12);
                    let bits = (0..20)
                        .map(|i| {
                            if (input as u32) >> i & 1 == 1 {
                                '1'
                            } else {
                                '0'
                            }
                        })
                        .collect::<String>();
                    let out = sim::basis(&f, &bits);
                    let expected = if steps < 3 {
                        (t as u32) | (((c ^ 1) as u32) << 8) | ((x as u32) << 12)
                    } else {
                        ((t ^ ((x + 1) & 255)) as u32) | ((c as u32) << 8) | ((x as u32) << 12)
                    };
                    let key = (0..20)
                        .map(|i| if expected >> i & 1 == 1 { '1' } else { '0' })
                        .collect::<String>();
                    assert_eq!(out.values().next().unwrap().value(), (1.0, 0.0));
                    assert_eq!(out.len(), 1);
                    assert_eq!(out[&key], exact::Amp::one());
                }
            }
        }
    }
}
