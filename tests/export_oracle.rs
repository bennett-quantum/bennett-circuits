//! Frozen literal construction: executable semantics and deterministic work counts.
//! Large-budget counts are projections, never completed exports. The Collatz
//! matrix exhausts x and probes outputs; it does not exhaust the 20-bit ABI.
use super::sibling;
use bennett_circuits::{
    flatten::Flat,
    ir::{BaseGate, Node, Program, QubitRef},
    json::Value,
    object::{Object, Operand},
    oracle::{Budget, specialise_legacy},
    qasm, sha256,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
    process::Command,
};

const COLLATZ: &str = include_str!("fixtures/export_oracle/collatz.bennett");
const COMMIT_GATES: usize = 15;
const CAPACITY_LIMITS: [(u64, u64); 13] = [
    (0, 0),
    (0, 1),
    (27, 0),
    (27, 1),
    (28, 0),
    (28, 1),
    (28, 2),
    (28, 3),
    (28, 4),
    (29, 0),
    (29, 1),
    (32, 0),
    (32, 1),
];
// Call admission, the first return (8), and the first retval (9).
const CAPACITY_STEPS: [u64; 6] = [0, 1, 7, 8, 9, 10];
const BASELINE_PREFIX: [usize; 9] = [92_331, 71, 111, 56, 277, 110, 340, 111_006, 238_029];

fn checked_collatz() -> Object {
    let hash = include_str!("fixtures/export_oracle/collatz.sha256")
        .split_whitespace()
        .next()
        .unwrap();
    assert_eq!(sha256::sha256_hex(COLLATZ.as_bytes()), hash);
    assert_eq!(COLLATZ.lines().count(), 54);
    let obj = Object::parse(COLLATZ).unwrap();
    let report = obj.check().unwrap();
    assert_eq!(report.status, "ok");
    assert!(report.unresolved_imports.is_empty());
    obj
}
fn budget(steps: u64, stack: u64, depth: u64) -> Budget {
    Budget {
        steps,
        heap: 0,
        stack,
        depth,
    }
}
fn legacy(obj: &Object, b: Budget) -> Program {
    specialise_legacy(obj, "ref", b, &[]).unwrap()
}

struct Flip {
    target: usize,
    controls: Range<usize>,
}
/// An independent phase-free evaluator. Each u64 holds 64 independent basis
/// tuples. It observes every allocated bit and never calls the product simulator.
struct Permutation {
    wires: usize,
    interface: usize,
    flips: Vec<Flip>,
    controls: Vec<(usize, bool)>,
}
impl Permutation {
    fn push(
        &mut self,
        gate: BaseGate,
        targets: &[usize],
        controls: &[(usize, bool)],
    ) -> Result<(), String> {
        if gate != BaseGate::X || targets.len() != 1 {
            return Err("permutation evaluator requires single-target X".into());
        }
        let target = targets[0];
        let mut used = BTreeSet::from([target]);
        if target >= self.wires
            || controls
                .iter()
                .any(|(q, _)| *q >= self.wires || !used.insert(*q))
        {
            return Err("out-of-range or overlapping executable operands".into());
        }
        let start = self.controls.len();
        self.controls.extend_from_slice(controls);
        self.flips.push(Flip {
            target,
            controls: start..self.controls.len(),
        });
        Ok(())
    }
    fn from_flat(flat: Flat) -> Result<Self, String> {
        if flat.interface != (0..flat.interface.len()).collect::<Vec<_>>() {
            return Err("evaluator expects contiguous interface".into());
        }
        let mut result = Self {
            wires: flat.num_qubits,
            interface: flat.interface.len(),
            flips: vec![],
            controls: vec![],
        };
        for gate in flat.gates {
            result.push(gate.gate, &gate.targets, &gate.controls)?;
        }
        Ok(result)
    }
    /// Direct literal evaluation avoids the legacy writer's N² circuit clones.
    /// Small circuits below additionally pass through actual executable QASM.
    fn from_literal(program: Program) -> Self {
        assert_eq!(program.circuits.len(), 1);
        let circuit = program.circuits.into_iter().next().unwrap();
        let mut map = BTreeMap::new();
        let mut wires = 0;
        for (name, width) in circuit.interface_bundles().into_iter().chain(
            circuit
                .workspace
                .iter()
                .map(|w| (w.name.clone(), u64::from(w.width))),
        ) {
            assert!(map.insert(name, (wires, width as usize)).is_none());
            wires += width as usize;
        }
        let resolve = |q: &QubitRef| {
            let (offset, width) = map[&q.bundle];
            assert!((q.index as usize) < width);
            offset + q.index as usize
        };
        let mut result = Self {
            wires,
            interface: circuit.interface_wire_count() as usize,
            flips: Vec::with_capacity(circuit.body.len()),
            controls: vec![],
        };
        for node in circuit.body {
            let Node::Gate(g) = node else {
                panic!("literal fixture must contain only gates")
            };
            let targets = g.targets.iter().map(&resolve).collect::<Vec<_>>();
            let controls = g
                .controls
                .iter()
                .map(|c| (resolve(&c.qubit), c.positive))
                .collect::<Vec<_>>();
            result.push(g.gate, &targets, &controls).unwrap();
        }
        result
    }
    /// Structural extension from output probes to arbitrary initial outputs:
    /// target/fault never feed a control or computation and are written only
    /// by X (XOR) outputs; input wires are never written. Private cleanup is
    /// checked separately on complete final states for every evaluated tuple.
    fn check_xor_outputs(&self, outputs: Range<usize>) -> Result<(), String> {
        for flip in &self.flips {
            if self.controls[flip.controls.clone()]
                .iter()
                .any(|(q, _)| outputs.contains(q))
            {
                return Err("target/fault feeds a control or computation".into());
            }
            if flip.target < self.interface && !outputs.contains(&flip.target) {
                return Err("read-only input is written".into());
            }
        }
        Ok(())
    }
    fn apply(&self, state: &mut [u64]) {
        assert_eq!(state.len(), self.wires);
        for flip in &self.flips {
            let enabled = self.controls[flip.controls.clone()]
                .iter()
                .fold(u64::MAX, |mask, (q, positive)| {
                    mask & if *positive { state[*q] } else { !state[*q] }
                });
            state[flip.target] ^= enabled;
        }
    }
}
fn packed(circuit: &Permutation, inputs: &[u64]) -> Vec<u64> {
    assert!(inputs.len() <= u64::BITS as usize);
    let mut state = vec![0; circuit.wires];
    for (lane, input) in inputs.iter().enumerate() {
        for (bit, word) in state.iter_mut().take(circuit.interface).enumerate() {
            *word |= ((input >> bit) & 1) << lane;
        }
    }
    state
}
fn assert_tuples(circuit: &Permutation, tuples: &[(u64, u64)]) {
    for batch in tuples.chunks(u64::BITS as usize) {
        let input = batch.iter().map(|(i, _)| *i).collect::<Vec<_>>();
        let output = batch.iter().map(|(_, o)| *o).collect::<Vec<_>>();
        let initial = packed(circuit, &input);
        let mut actual = initial.clone();
        circuit.apply(&mut actual);
        // Compare every private bit as well as the whole interface. Unused
        // lanes are intentionally masked, since negative controls activate them.
        let mask = u64::MAX >> (u64::BITS as usize - batch.len());
        for (wire, expected) in packed(circuit, &output).into_iter().enumerate() {
            assert_eq!(
                actual[wire] & mask,
                expected & mask,
                "wire={wire}, first input={}",
                input[0]
            );
        }
        circuit.apply(&mut actual);
        for (wire, expected) in initial.into_iter().enumerate() {
            assert_eq!(
                actual[wire] & mask,
                expected & mask,
                "involution wire={wire}"
            );
        }
    }
}
fn collatz_probes() -> Vec<(u64, u64)> {
    [(0, 0), (255, 15), (85, 10), (170, 5)]
        .into_iter()
        .chain((0..8).map(|i| (1 << i, 0)))
        .chain((0..4).map(|i| (0, 1 << i)))
        .collect()
}
fn collatz_tuples(b: Budget) -> Vec<(u64, u64)> {
    let mut tuples = vec![];
    for x in 0..256u64 {
        // Hand-derived original-row boundaries: the sole success up to B=9
        // is x=1 at row charge 9, with payload zero. Call entry needs 28 cells.
        let code = if b.steps == 0 {
            1
        } else if b.stack < 28 || b.depth == 0 {
            3
        } else if b.steps >= 9 && x == 1 {
            0
        } else {
            1
        };
        for (t, f) in collatz_probes() {
            let input = t | (f << 8) | (x << 12);
            let output = t | ((f ^ code) << 8) | (x << 12);
            tuples.push((input, output));
        }
    }
    tuples
}
fn histogram(nodes: &[Node]) -> BTreeMap<usize, usize> {
    let mut hist = BTreeMap::new();
    for node in nodes {
        let Node::Gate(g) = node else {
            panic!("expected primitive gate")
        };
        assert_eq!(g.gate, BaseGate::X);
        assert_eq!(g.targets.len(), 1);
        *hist.entry(g.controls.len()).or_insert(0) += 1;
    }
    hist
}
fn forward_counts(program: &Program) -> Vec<usize> {
    let c = program.top_circuit().unwrap();
    assert_eq!(program.circuits.len(), 1);
    let forward = (c.body.len() - COMMIT_GATES) / 2;
    assert_eq!(c.body.len(), 2 * forward + COMMIT_GATES);
    for (a, b) in c.body[..forward]
        .iter()
        .rev()
        .zip(&c.body[forward + COMMIT_GATES..])
    {
        assert_eq!(*a, b.adjoint());
    }
    assert_eq!(
        histogram(&c.body[forward..forward + COMMIT_GATES]),
        BTreeMap::from([(1, 1), (2, 4), (3, 8), (6, 2)])
    );
    let dispatch = c
        .workspace
        .iter()
        .filter(|w| w.name.starts_with("$dispatch"))
        .map(|w| w.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut starts = vec![];
    for (i, node) in c.body[..forward].iter().enumerate() {
        let Node::Gate(g) = node else {
            panic!("expected gate")
        };
        let target = g.targets[0].bundle.as_str();
        if dispatch.contains(target) && seen.insert(target) {
            starts.push(i);
        }
    }
    assert_eq!(starts.first(), Some(&0)); // This fixture has zero setup gates.
    let last = *starts.last().unwrap();
    if starts.len() == BASELINE_PREFIX.len() {
        assert_eq!(
            histogram(&c.body[last..forward]),
            BTreeMap::from([(1, 30_446), (2, 32_763), (3, 176), (4, 174_576), (6, 68)])
        );
    }
    starts.push(forward);
    starts.windows(2).map(|w| w[1] - w[0]).collect()
}
/// Independent transcription of the *legacy reachability overapproximation*,
/// not a semantic interpreter. It justifies the steady full-layer projection:
/// after return every row and HALT are possible, and this set is invariant.
fn assert_steady_legacy_rows(obj: &Object) {
    let functions = [obj.function("ref").unwrap(), obj.function("L1").unwrap()];
    let mut rows = vec![];
    let mut labels = BTreeMap::new();
    let mut entries = BTreeMap::new();
    for f in functions {
        entries.insert(f.name.as_str(), rows.len());
        for block in &f.blocks {
            labels.insert((f.name.as_str(), block.name.as_str()), rows.len());
            for i in &block.instrs {
                rows.push((f.name.as_str(), i));
            }
        }
    }
    let halt = rows.len();
    assert_eq!(halt, 33);
    let label = |f, operand: &Operand| {
        let Operand::Label(name, _) = operand else {
            panic!("expected label")
        };
        labels[&(f, name.as_str())]
    };
    let mut possible = BTreeSet::from([0]);
    for step in 1..=10 {
        if step >= 9 {
            assert_eq!(possible, (0..=halt).collect());
        }
        let mut next = BTreeSet::new();
        for k in possible {
            if k == halt {
                next.insert(halt);
                continue;
            }
            let (f, row) = rows[k];
            let successors = match row.opcode.as_str() {
                "qjump" => vec![label(f, &row.operands[0])],
                "qbranch" => vec![label(f, &row.operands[1]), label(f, &row.operands[2])],
                "lift.call" => {
                    let Operand::Sym(name) = &row.operands[0] else {
                        panic!("expected callee")
                    };
                    vec![entries[name.as_str()], halt]
                }
                "lift.ret" => (0..=halt).collect(),
                "lift.retval" => vec![halt],
                _ => vec![k + 1, halt],
            };
            next.extend(successors);
        }
        possible = next;
    }
}
fn projected_gates(prefix: &[usize], steps: usize) -> usize {
    assert_eq!(prefix.len(), 9);
    2 * (prefix[..steps.min(8)].iter().sum::<usize>() + steps.saturating_sub(8) * prefix[8])
        + COMMIT_GATES
}

#[test]
fn checked_collatz_fixture_is_hash_bound() {
    let obj = checked_collatz();
    let p = legacy(&obj, budget(0, 32, 32));
    assert_eq!(p.top_circuit().unwrap().body.len(), COMMIT_GATES);
}
#[test]
fn collatz_legacy_counts_semantics_cleanup_and_involution() {
    let obj = checked_collatz();
    assert_steady_legacy_rows(&obj);
    for (steps, expected) in [
        (0, 15),
        (1, 184_677),
        (2, 184_819),
        (4, 185_153),
        (8, 408_619),
        (9, 884_677),
    ] {
        let b = budget(steps, 32, 32);
        let p = legacy(&obj, b);
        assert_eq!(p.top_circuit().unwrap().body.len(), expected);
        if steps > 0 {
            let prefix = forward_counts(&p);
            assert_eq!(prefix, BASELINE_PREFIX[..steps as usize]);
            if steps == 9 {
                assert_eq!(2 * prefix[8], 476_058);
                for (n, count) in [(16, 4_217_083), (32, 11_834_011), (2048, 971_566_939)] {
                    assert_eq!(projected_gates(&prefix, n), count);
                }
                // Expected-failing stage-3 size obligations, kept as green
                // baseline evidence. Later stages must enable real ceilings.
                assert!(projected_gates(&prefix, 1) > 2_000 + 1_000);
                assert!(projected_gates(&prefix, 32) > 2_000 * 32 + 1_000);
            }
        }
        let executable = Permutation::from_literal(p);
        assert_eq!(executable.interface, 20);
        executable.check_xor_outputs(0..12).unwrap();
        assert_tuples(&executable, &collatz_tuples(b));
    }
}
#[test]
fn collatz_call_capacity_boundaries_preserve_first_fault() {
    let obj = checked_collatz();
    for (stack, depth) in CAPACITY_LIMITS {
        for steps in CAPACITY_STEPS {
            let b = budget(steps, stack, depth);
            eprintln!("checking call capacity {b:?}");
            let p = Permutation::from_literal(legacy(&obj, b));
            p.check_xor_outputs(0..12).unwrap();
            assert_tuples(&p, &collatz_tuples(b));
        }
    }
}
#[test]
fn collatz_cli_zero_budget_executes_without_ir_metadata() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/export_oracle/collatz.bennett");
    let out = Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
        .args([
            "export-qasm",
            fixture.to_str().unwrap(),
            "ref",
            "--budget",
            "0,0,32,32",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let executable = text
        .lines()
        .filter(|l| !l.starts_with("// bc:"))
        .collect::<Vec<_>>()
        .join("\n");
    let parsed = Permutation::from_flat(qasm::parse_flat(&executable).unwrap()).unwrap();
    assert_eq!(parsed.flips.len(), COMMIT_GATES);
    parsed.check_xor_outputs(0..12).unwrap();
    assert_tuples(&parsed, &collatz_tuples(budget(0, 32, 32)));
}
#[test]
fn small_oracle_exhausts_entire_abi_through_executable_qasm() {
    // Hand-derived two-bit increment modulo four; three original source rows.
    let object = Object::parse(include_str!("fixtures/export_oracle/increment2.bennett")).unwrap();
    object.check().unwrap();
    for steps in 0..=12 {
        let p = specialise_legacy(&object, "inc2", budget(steps, 0, 0), &[]).unwrap();
        let text = qasm::export(&p).unwrap();
        let executable = text
            .lines()
            .filter(|l| !l.starts_with("// bc:"))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = Permutation::from_flat(qasm::parse_flat(&executable).unwrap()).unwrap();
        assert_eq!(parsed.interface, 8);
        parsed.check_xor_outputs(0..6).unwrap();
        let tuples = (0..256)
            .map(|input| {
                let (target, fault, x) = (input & 3, (input >> 2) & 15, input >> 6);
                let (payload, code) = if steps < 3 { (0, 1) } else { ((x + 1) & 3, 0) };
                (input, (target ^ payload) | ((fault ^ code) << 2) | (x << 6))
            })
            .collect::<Vec<_>>();
        assert_tuples(&parsed, &tuples);
    }
}
#[test]
fn evaluator_and_structural_checks_reject_corruptions() {
    let text = "OPENQASM 3.0;\ninclude \"stdgates.inc\";\n// interface: 3\ngate toggles v0, v1, v2 {\n  ctrl @ negctrl @ x v1, v2, v0;\n}\nqubit[3] q;\ntoggles q[0], q[1], q[2];\n";
    let correct = Permutation::from_flat(qasm::parse_flat(text).unwrap()).unwrap();
    correct.check_xor_outputs(0..1).unwrap();
    let tuples = (0..8)
        .map(|i| (i, i ^ if i & 2 != 0 && i & 4 == 0 { 1 } else { 0 }))
        .collect::<Vec<_>>();
    assert_tuples(&correct, &tuples);
    let bad_gate = text.replace("@ x", "@ h");
    assert!(Permutation::from_flat(qasm::parse_flat(&bad_gate).unwrap()).is_err());
    for output in [0, 1] {
        // Deliberately feed target, then fault into a private flip.
        let mut corrupt = Permutation {
            wires: 4,
            interface: 3,
            flips: vec![],
            controls: vec![],
        };
        corrupt.push(BaseGate::X, &[3], &[(output, true)]).unwrap();
        assert!(
            corrupt
                .check_xor_outputs(0..2)
                .unwrap_err()
                .contains("target/fault")
        );
    }
    let mut dirty = Permutation {
        wires: 4,
        interface: 3,
        flips: vec![],
        controls: vec![],
    };
    dirty.push(BaseGate::X, &[3], &[]).unwrap();
    let mut state = packed(&dirty, &[0]);
    dirty.apply(&mut state);
    assert_ne!(state, packed(&dirty, &[0])); // Sees private dirt even with equal interfaces.
    let mut input_write = Permutation {
        wires: 4,
        interface: 3,
        flips: vec![],
        controls: vec![],
    };
    input_write.push(BaseGate::X, &[2], &[]).unwrap();
    assert!(
        input_write
            .check_xor_outputs(0..2)
            .unwrap_err()
            .contains("read-only")
    );
    let polarity =
        Permutation::from_flat(qasm::parse_flat(&text.replace("negctrl", "ctrl")).unwrap())
            .unwrap();
    let mut a = packed(&correct, &[2]);
    let mut b = a.clone();
    correct.apply(&mut a);
    polarity.apply(&mut b);
    assert_ne!(a, b);
}
#[test]
fn pinned_reference_adjudicates_collatz_row_and_capacity_boundaries() {
    let binary = sibling::isa().join("reference-interpreter/target/debug/bennett-isa-ref");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/export_oracle/collatz.bennett");
    let capacity_cases = CAPACITY_LIMITS.into_iter().flat_map(|(stack, depth)| {
        CAPACITY_STEPS.into_iter().flat_map(move |steps| {
            [0, 1, 2, 255].map(|x| {
                let (code, executed) = if steps == 0 {
                    (1, 0)
                } else if stack < 28 || depth == 0 {
                    (3, 1)
                } else if x == 1 && steps >= 9 {
                    (0, 9)
                } else {
                    (1, steps)
                };
                (x, steps, stack, depth, 0, code, executed)
            })
        })
    });
    for (x, steps, stack, depth, payload, code, executed) in [
        (1, 8, 32, 32, 0, 1, 8),
        (1, 9, 32, 32, 0, 0, 9),
        (2, 27, 32, 32, 0, 1, 27),
        (2, 28, 32, 32, 1, 0, 28),
        (4, 45, 32, 32, 0, 1, 45),
        (4, 46, 32, 32, 2, 0, 46),
        (0, 2048, 32, 32, 0, 1, 2048),
        (1, 0, 27, 1, 0, 1, 0),
        (1, 1, 27, 1, 0, 3, 1),
        (1, 1, 28, 0, 0, 3, 1),
        (1, 9, 28, 1, 0, 0, 9),
    ]
    .into_iter()
    .chain(capacity_cases)
    {
        let out = Command::new(&binary)
            .args([
                "run-oracle",
                fixture.to_str().unwrap(),
                "ref",
                "--input",
                &x.to_string(),
                "--budget",
                &format!("{steps},0,{stack},{depth}"),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let result = Value::parse(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
        for (key, expected) in [("payload", payload), ("code", code), ("steps", executed)] {
            assert_eq!(
                result.get(key).and_then(Value::as_u64),
                Some(expected),
                "x={x} B={steps} {key}"
            );
        }
    }
}
