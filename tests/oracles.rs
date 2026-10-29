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
                        .map(|i| if input >> i & 1 == 1 { '1' } else { '0' })
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

#[test]
fn every_oracle_matches_cli_on_basis_tuples() {
    use bennett_circuits::{
        json::Value,
        object::{FnKind, Type},
        oracle::Argument,
    };
    use std::{
        path::{Path, PathBuf},
        process::Command,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = root.parent().unwrap();
    let binary =
        workspace.join("bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref");
    let mut paths =
        std::fs::read_dir(workspace.join("bennett-isa-spec/reference-interpreter/tests/fixtures"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "bisa"))
            .collect::<Vec<PathBuf>>();
    paths.sort();
    let dir = root.join(".tmp/oracles");
    std::fs::create_dir_all(&dir).unwrap();
    let spec = std::fs::read_to_string(workspace.join("bennett-isa-spec/spec/Bennett-ISA-Spec.md"))
        .unwrap();
    for (n, block) in spec
        .split("```")
        .filter(|b| b.trim_start().starts_with("bennett-object "))
        .enumerate()
    {
        let p = dir.join(format!("example{n}.bisa"));
        std::fs::write(&p, block.trim_start()).unwrap();
        paths.push(p);
    }
    let (mut count, mut cases) = (0, 0);
    for path in paths {
        let o = Object::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for fun in o.functions.iter().filter(|f| f.kind == FnKind::Oracle) {
            let budget = Budget {
                steps: if fun.name == "sum_db" {
                    12
                } else {
                    fun.blocks.iter().map(|b| b.instrs.len()).sum::<usize>() as u64 + 6
                },
                heap: o
                    .layout
                    .as_ref()
                    .and_then(|v| v.get("heap_limit_max"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(32),
                stack: if let Some(layout) = &o.layout {
                    64.min(1 << layout.get("S_cells").unwrap().as_u64().unwrap())
                } else {
                    0
                },
                depth: if o.layout.is_some() { 4 } else { 0 },
            };
            let args = fun.groups[4]
                .iter()
                .map(|p| match p.ty {
                    Type::Token => Argument::Token(Budget {
                        steps: 4,
                        heap: 0,
                        stack: 0,
                        depth: 0,
                    }),
                    Type::Int(_) => Argument::Integer("1".into()),
                    _ => panic!("test argument type"),
                })
                .collect::<Vec<_>>();
            let p = specialise(&o, &fun.name, budget, &args)
                .unwrap_or_else(|e| panic!("{} @{}: {e}", path.display(), fun.name));
            let flat = flatten(&p).unwrap();
            let w = flat.interface.len();
            eprintln!(
                "CIRCUIT {} @{}: {} wires {} gates",
                path.display(),
                fun.name,
                flat.num_qubits,
                flat.gates.len()
            );
            let domain = if w <= 10 { 1usize << w } else { 4 };
            for column in 0..domain {
                let mut bits = vec![b'0'; w];
                for (i, b) in bits.iter_mut().enumerate() {
                    *b = if w <= 10 {
                        if column >> i & 1 == 1 { b'1' } else { b'0' }
                    } else {
                        match column {
                            0 => b'0',
                            1 => b'1',
                            2 => {
                                if i % 2 == 0 {
                                    b'1'
                                } else {
                                    b'0'
                                }
                            }
                            _ => {
                                if i % 3 == 0 {
                                    b'1'
                                } else {
                                    b'0'
                                }
                            }
                        }
                    };
                }
                let to_decimal = |slice: &[u8]| {
                    let mut digits = vec![0u8];
                    for bit in slice.iter().rev() {
                        let mut carry = if *bit == b'1' { 1 } else { 0 };
                        for d in &mut digits {
                            let v = *d * 2 + carry;
                            *d = v % 10;
                            carry = v / 10;
                        }
                        if carry > 0 {
                            digits.push(carry)
                        }
                    }
                    digits
                        .iter()
                        .rev()
                        .map(|d| (b'0' + d) as char)
                        .collect::<String>()
                };
                let target = fun.groups[1][0].ty.wire_width().unwrap() as usize;
                let fault = fun.groups[2][0].ty.wire_width().unwrap() as usize;
                let mut cursor = target + fault;
                let mut inputs = Vec::new();
                for param in &fun.groups[0] {
                    let n = param.ty.wire_width().unwrap() as usize;
                    inputs.push(to_decimal(&bits[cursor..cursor + n]));
                    cursor += n;
                }
                let mut cmd = Command::new(&binary);
                cmd.args([
                    "run-oracle",
                    path.to_str().unwrap(),
                    &fun.name,
                    "--input",
                    &inputs.join(","),
                    "--budget",
                    &format!(
                        "{},{},{},{}",
                        budget.steps, budget.heap, budget.stack, budget.depth
                    ),
                ]);
                for param in &fun.groups[3] {
                    let Type::Region(a, cw) = param.ty else {
                        unreachable!()
                    };
                    let n = (1usize << a) * cw as usize;
                    let hex = bits[cursor..cursor + n]
                        .chunks(8)
                        .map(|c| {
                            let v = c
                                .iter()
                                .enumerate()
                                .fold(0u8, |v, (i, b)| v | if *b == b'1' { 1 << i } else { 0 });
                            format!("{v:02x}")
                        })
                        .collect::<String>();
                    cmd.args(["--mem", &format!("{}={hex}", param.name)]);
                    cursor += n;
                }
                for arg in &args {
                    let value = match arg {
                        Argument::Token(t) => {
                            format!("token:{},{},{},{}", t.steps, t.heap, t.stack, t.depth)
                        }
                        Argument::Integer(n) => n.clone(),
                        _ => unreachable!(),
                    };
                    cmd.args(["--arg", &value]);
                }
                let out = cmd
                    .env_remove("DISPLAY")
                    .env_remove("WAYLAND_DISPLAY")
                    .env_remove("DBUS_SESSION_BUS_ADDRESS")
                    .env_remove("BROWSER")
                    .env("TMPDIR", root.join(".tmp"))
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                let result = Value::parse(&String::from_utf8(out.stdout).unwrap()).unwrap();
                let mut expected = bits.clone();
                for (key, offset, n) in [("payload", 0, target), ("code", target, fault)] {
                    let value = result.get(key).unwrap();
                    let decimal = match value {
                        Value::Int(s) | Value::Str(s) => s,
                        _ => panic!("integer outcome"),
                    };
                    let mut digits = decimal.bytes().map(|d| d - b'0').collect::<Vec<_>>();
                    for j in 0..n {
                        let mut carry = 0;
                        for d in &mut digits {
                            let x = carry * 10 + *d;
                            *d = x / 2;
                            carry = x % 2;
                        }
                        if carry == 1 {
                            expected[offset + j] = if expected[offset + j] == b'0' {
                                b'1'
                            } else {
                                b'0'
                            };
                        }
                    }
                }
                let input = String::from_utf8(bits).unwrap();
                let expected = String::from_utf8(expected).unwrap();
                let ours = sim::basis(&flat, &input);
                assert_eq!(ours.len(), 1, "{} @{} {input}", path.display(), fun.name);
                assert_eq!(
                    ours.get(&expected),
                    Some(&exact::Amp::one()),
                    "DIVERGENCE {} @{} input={input} expected={expected} reference={result:?}",
                    path.display(),
                    fun.name
                );
                cases += 1;
            }
            count += 1;
        }
    }
    eprintln!("ORACLE HARNESS: {count} oracles, {cases} tuples, {cases} agree / 0 diverge");
}
#[test]
fn allocator_assignment_slots_follow_spec_order() {
    let o=Object::parse(&std::fs::read_to_string("/home/tobias/Projects/bennett-workspace/bennett-isa-spec/reference-interpreter/tests/fixtures/heap.bisa").unwrap()).unwrap();
    let p = specialise(
        &o,
        "heap",
        Budget {
            steps: 2,
            heap: 32,
            stack: 0,
            depth: 0,
        },
        &[],
    )
    .unwrap();
    let c = p.top_circuit().unwrap();
    let names = c
        .workspace
        .iter()
        .map(|w| w.name.as_str())
        .collect::<Vec<_>>();
    let w = names
        .iter()
        .position(|n| n.starts_with("$alloc_disp_w"))
        .expect("§4.2 alloc destination assignment slot");
    let table = names
        .iter()
        .position(|n| n.starts_with("$alloc_entry"))
        .unwrap();
    let heap = names
        .iter()
        .position(|n| n.starts_with("$alloc_disp_heap"))
        .unwrap();
    let atop = names
        .iter()
        .position(|n| n.starts_with("$alloc_disp_atop"))
        .unwrap();
    assert!(
        w < table && table < heap && heap < atop,
        "§4.2 four assignment slots in listed order"
    );
}
