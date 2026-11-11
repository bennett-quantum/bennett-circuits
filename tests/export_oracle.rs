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

#[test]
fn analyzed_plan_binds_identity_constants_and_call_contexts() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, Check, Event, analyze};
    let obj = checked_collatz();
    let b = budget(2048, 32, 32);
    let plan = analyze(&obj, "ref", b, &[], &[], AnalysisOrder::Forward).unwrap();
    assert_eq!(plan.binding().object_hash, obj.object_hash());
    assert_eq!(plan.binding().budget, b);
    assert!(!plan.is_general());
    assert_eq!(plan.rows().len(), 33);
    let entry = plan.node(0, &[]).unwrap();
    assert!(matches!(
        entry.event,
        Event::Call {
            cells: 28,
            check: Check::Safe,
            ..
        }
    ));
    let ret = plan.node(32, &[0]).unwrap();
    assert_eq!(
        ret.successors
            .iter()
            .filter_map(|e| e.target.as_ref())
            .map(|k| k.pc)
            .collect::<Vec<_>>(),
        [1]
    );
    let mul = plan.node(19, &[0]).unwrap();
    assert_eq!(mul.operands[1].as_u128(), Some(3));
    let shift = plan.node(17, &[0]).unwrap();
    assert_eq!(shift.operands[1].as_u128(), Some(1));
    assert!(plan.layers().descriptor_count() < 256);
    assert!(
        plan.layer(2047)
            .iter()
            .all(|k| k.pc == plan.rows().len() || k.calls == [0] || k.pc == 1)
    );
}

#[test]
fn analyzed_plan_fixpoint_is_independent_of_function_and_block_visits() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    let obj = checked_collatz();
    let forward = analyze(
        &obj,
        "ref",
        budget(2048, 32, 32),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    for order in [
        AnalysisOrder::FunctionsReversed,
        AnalysisOrder::BlocksReversed,
    ] {
        let other = analyze(&obj, "ref", budget(2048, 32, 32), &[], &[], order).unwrap();
        assert_eq!(forward.rows(), other.rows());
        assert_eq!(forward.nodes(), other.nodes());
        assert_eq!(forward.layers(), other.layers());
        assert_eq!(forward.binding(), other.binding());
    }
}

#[test]
fn analyzed_plan_literals_preserve_modular_negative_and_wide_values() {
    use bennett_circuits::oracle_plan::KnownBits;
    for width in 1..=8 {
        for value in -255i32..=255 {
            let bits = KnownBits::integer(&value.to_string(), width);
            assert_eq!(
                bits.as_u128(),
                Some((value as u128) & ((1 << width) - 1)),
                "value={value} width={width}"
            );
        }
    }
    let minus_one = KnownBits::integer("-1", 257);
    assert!(minus_one.0.iter().all(|b| *b == Some(true)));
    assert_eq!(minus_one.as_u128(), None);
}

fn plan_object(source: &str) -> Object {
    use bennett_circuits::object::{Symbol, derived_signature};
    let mut object = Object::parse(source).unwrap();
    if !object.symbols.is_empty() {
        object.check().unwrap();
        return object;
    }
    object.symbols = object
        .functions
        .iter()
        .enumerate()
        .map(|(index, f)| Symbol {
            index: index.to_string(),
            name: f.name.clone(),
            kind: f.kind.symbol_kind().into(),
            signature: derived_signature(f).unwrap(),
            stored_interface_hash: String::new(),
        })
        .collect();
    let checked = object.materialize().unwrap();
    checked.check().unwrap();
    checked
}

#[test]
fn analyzed_plan_call_sites_keep_monotonic_stack_and_saved_locations() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, Check, Event, analyze};
    let object = plan_object(include_str!("fixtures/export_oracle/plan/calls.bennett"));
    for (stack, depth, second) in [
        (5, 1, None),
        (6, 0, None),
        (6, 1, Some(Check::Fault)),
        (11, 1, Some(Check::Fault)),
        (12, 1, Some(Check::Safe)),
    ] {
        let plan = analyze(
            &object,
            "calls",
            budget(64, stack, depth),
            &[],
            &[],
            AnalysisOrder::Forward,
        )
        .unwrap();
        let entry = plan.node(0, &[]).unwrap();
        let first = if stack < 6 || depth == 0 {
            Check::Fault
        } else {
            Check::Safe
        };
        assert!(matches!(entry.event, Event::Call { cells: 6, check, .. } if check == first));
        match second {
            None => assert!(plan.node(1, &[]).is_none()),
            Some(expected) => {
                assert!(
                    matches!(plan.node(1, &[]).unwrap().event, Event::Call { stack: s, depth: Some(0), check, .. } if s.min == 6 && s.max == 6 && check == expected)
                );
                let ret = plan.node(5, &[0]).unwrap();
                assert_eq!(ret.successors[0].target.as_ref().unwrap().pc, 1);
            }
        }
        if second == Some(Check::Safe) {
            let one = plan
                .locations()
                .iter()
                .position(|l| l.function == "inc" && l.name == "one")
                .unwrap();
            // Each fresh activation is zero even after a previous call used one=1.
            assert_eq!(plan.node(3, &[1]).unwrap().values[one].as_u128(), Some(0));
            assert_eq!(
                plan.node(5, &[1]).unwrap().successors[0]
                    .target
                    .as_ref()
                    .unwrap()
                    .pc,
                2
            );
        }
    }
}

#[test]
fn analyzed_plan_join_preserves_only_common_bits_and_canonical_labels() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    let object = plan_object(include_str!("fixtures/export_oracle/plan/diamond.bennett"));
    let plan = analyze(
        &object,
        "diamond",
        budget(12, 0, 0),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    let joined = plan.node(6, &[]).unwrap();
    assert_eq!(joined.operands[1].0, [Some(true), None]);
    assert_eq!(plan.node(8, &[]).unwrap().operands[0].as_u128(), Some(11));
    assert_eq!(
        plan.node(8, &[]).unwrap().successors[0]
            .target
            .as_ref()
            .unwrap()
            .pc,
        11
    );
    assert!(plan.node(9, &[]).is_none());
    for order in [
        AnalysisOrder::FunctionsReversed,
        AnalysisOrder::BlocksReversed,
    ] {
        let other = analyze(&object, "diamond", budget(12, 0, 0), &[], &[], order).unwrap();
        assert_eq!(plan.nodes(), other.nodes());
        assert_eq!(plan.layers(), other.layers());
    }
}

#[test]
fn analyzed_plan_recursion_uses_a_general_plan_without_losing_rows() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, Check, Event, analyze};
    let object = plan_object(include_str!(
        "fixtures/export_oracle/plan/recursive.bennett"
    ));
    let plan = analyze(
        &object,
        "recurse",
        budget(u64::MAX, 16, 16),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    assert!(plan.is_general());
    assert_eq!(plan.nodes().len(), plan.rows().len());
    assert!(matches!(
        plan.node(2, &[]).unwrap().event,
        Event::Call {
            check: Check::Dynamic,
            depth: None,
            ..
        }
    ));
    assert_eq!(plan.layer(u64::MAX).len(), plan.rows().len() + 1);
    assert_eq!(plan.layers().descriptor_count(), 2);
    let mut nodes = plan.nodes().values();
    let first = nodes.next().unwrap();
    for node in nodes {
        assert!(std::sync::Arc::ptr_eq(&first.values, &node.values));
        assert!(std::sync::Arc::ptr_eq(&first.successors, &node.successors));
    }
    assert!(
        plan.nodes()
            .values()
            .all(|n| n.general_kernel && n.source_charge == 1)
    );
}

// Concrete original-row execution, independent of the abstract transfers. Checks
// every reached row/context, all known bits and virtual capacity against the plan.
fn check_collatz_trace(
    plan: &bennett_circuits::oracle_plan::Plan,
    object: &Object,
    input: u128,
) -> (u128, u8, u64) {
    use bennett_circuits::oracle_plan::{Check, Event, RowKey};
    let rows = [
        object.function("ref").unwrap(),
        object.function("L1").unwrap(),
    ]
    .into_iter()
    .flat_map(|f| {
        f.blocks
            .iter()
            .flat_map(move |b| b.instrs.iter().map(move |i| (f.name.as_str(), i)))
    })
    .collect::<Vec<_>>();
    let mut values = plan
        .locations()
        .iter()
        .map(|l| ((l.function.clone(), l.name.clone()), 0u128))
        .collect::<BTreeMap<_, _>>();
    *values.get_mut(&("ref".into(), "x0".into())).unwrap() = input;
    let labels = object
        .functions
        .iter()
        .flat_map(|f| {
            f.blocks
                .iter()
                .map(move |b| (f.name.clone(), b.name.clone()))
        })
        .map(|(f, b)| {
            let pc = plan
                .rows()
                .iter()
                .find(|r| r.function == f && r.block == b)
                .unwrap()
                .pc;
            ((f, b), pc)
        })
        .collect::<BTreeMap<_, _>>();
    let (mut pc, mut stack) = (0, 0u64);
    let mut calls = vec![];
    let mut saved = None;
    let b = plan.binding().budget;
    for step in 0..b.steps {
        let key = RowKey {
            pc,
            calls: calls.clone(),
        };
        assert!(plan.layer(step).contains(&key), "missing {key:?} at {step}");
        let node = plan.node(pc, &calls).unwrap();
        for (location, fact) in plan.locations().iter().zip(node.values.iter()) {
            assert!(
                fact.contains(values[&(location.function.clone(), location.name.clone())]),
                "unsound {key:?} {location:?} input={input}"
            );
        }
        let (f, i) = rows[pc];
        let read = |operand: &Operand| match operand {
            Operand::Name(n) => values[&(f.into(), n.clone())],
            Operand::Int(s) => s.parse::<u128>().unwrap(),
            _ => panic!("concrete test read {operand:?}"),
        };
        let label = |operand: &Operand| {
            let Operand::Label(n, _) = operand else {
                panic!("label")
            };
            labels[&(f.into(), n.clone())]
        };
        let op = i.opcode.strip_prefix("lift.").unwrap_or(&i.opcode);
        let mut next = pc + 1;
        let mut write = None;
        match op {
            "call" => {
                let failed = stack + 28 > b.stack || calls.len() as u64 >= b.depth;
                assert!(
                    matches!(node.event, Event::Call { stack: s, check, .. } if s.min <= stack && stack <= s.max && (check != Check::Safe || !failed) && (check != Check::Fault || failed))
                );
                if failed {
                    assert!(
                        node.successors
                            .iter()
                            .any(|e| e.target.is_none() && e.fault == Some(Some(3)))
                    );
                    return (0, 3, step + 1);
                }
                let argument = values[&("ref".into(), "x0".into())];
                saved = Some(values.clone());
                for ((function, _), v) in &mut values {
                    if function == "L1" {
                        *v = 0;
                    }
                }
                *values.get_mut(&("L1".into(), "a0".into())).unwrap() = argument;
                stack += 28;
                calls.push(pc);
                next = 2;
            }
            "ret" => {
                let value = read(&i.operands[0]);
                let old = saved.take().unwrap();
                for ((function, n), v) in &mut values {
                    if function == "L1" {
                        *v = old[&(function.clone(), n.clone())];
                    }
                }
                *values.get_mut(&("ref".into(), "r".into())).unwrap() = value;
                next = calls.pop().unwrap() + 1;
            }
            "retval" => {
                assert!(
                    node.successors
                        .iter()
                        .any(|e| e.target.is_none() && e.fault.is_none())
                );
                return (read(&i.operands[0]), 0, step + 1);
            }
            "qjump" => next = label(&i.operands[0]),
            "qbranch" => {
                next = label(&i.operands[if read(&i.operands[0]) & 1 != 0 { 1 } else { 2 }])
            }
            "parmov" => {
                let Operand::List(names) = &i.operands[0] else {
                    panic!("parmov")
                };
                let new = names
                    .iter()
                    .map(|n| values[&(f.into(), n.clone())])
                    .collect::<Vec<_>>();
                for (n, v) in i.lhs.iter().zip(new) {
                    *values.get_mut(&(f.into(), n.clone())).unwrap() = v;
                }
            }
            _ if op.starts_with("const.i") || op.starts_with("zext.i") => {
                write = Some(read(&i.operands[0]))
            }
            "icmp.eq" => write = Some(u128::from(read(&i.operands[0]) == read(&i.operands[1]))),
            "and" => write = Some(read(&i.operands[0]) & read(&i.operands[1])),
            "add" => write = Some(read(&i.operands[0]).wrapping_add(read(&i.operands[1]))),
            "mul" => write = Some(read(&i.operands[0]).wrapping_mul(read(&i.operands[1]))),
            "lshr" => write = Some(read(&i.operands[0]) >> read(&i.operands[1])),
            "select" => {
                write = Some(read(
                    &i.operands[if read(&i.operands[0]) & 1 != 0 { 1 } else { 2 }],
                ))
            }
            _ => panic!("concrete Collatz test opcode {op}"),
        }
        if let Some(value) = write {
            let n = &i.lhs[0];
            let width = plan
                .locations()
                .iter()
                .find(|l| l.function == f && l.name == *n)
                .unwrap()
                .width;
            *values.get_mut(&(f.into(), n.clone())).unwrap() = value & ((1u128 << width) - 1);
        }
        let destination = RowKey {
            pc: next,
            calls: calls.clone(),
        };
        assert!(
            node.successors
                .iter()
                .any(|e| e.target.as_ref() == Some(&destination)),
            "missing edge {key:?} -> {destination:?}"
        );
        pc = next;
    }
    assert_eq!(plan.boundary_fault(&RowKey { pc, calls }), Some(1));
    (0, 1, b.steps)
}

#[test]
fn analyzed_plan_covers_all_collatz_inputs_at_original_budget_positions() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    let object = checked_collatz();
    for steps in [0, 1, 2, 4, 8, 9, 10, 27, 28, 29, 45, 46, 47, 64, 2048] {
        let plan = analyze(
            &object,
            "ref",
            budget(steps, 32, 32),
            &[],
            &[],
            AnalysisOrder::Forward,
        )
        .unwrap();
        for input in 0..256 {
            let result = check_collatz_trace(&plan, &object, input);
            if input == 0 {
                assert_eq!(result, (0, 1, steps));
            }
            for (x, first, payload) in [(1, 9, 0), (2, 28, 1), (4, 46, 2)] {
                if input == x && steps >= first {
                    assert_eq!(result, (payload, 0, first));
                }
            }
        }
    }
    for (stack, depth) in CAPACITY_LIMITS {
        for steps in CAPACITY_STEPS {
            let plan = analyze(
                &object,
                "ref",
                budget(steps, stack, depth),
                &[],
                &[],
                AnalysisOrder::Forward,
            )
            .unwrap();
            for input in [0, 1, 2, 255] {
                check_collatz_trace(&plan, &object, input);
            }
        }
    }
}

fn tiny_plan_program(regs: &str, body: &str, input: &str, arguments: &str) -> Object {
    plan_object(&format!(
        "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\noracle @tiny ({input}) (t:wire[4]) (f:wire[4]) () ({arguments})\nregs {{ {regs} }}\n^entry():\n{body}\n"
    ))
}

#[test]
fn analyzed_plan_constant_and_partial_arithmetic_matches_all_small_inputs() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    for op in [
        "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr",
    ] {
        for constant in 0..=5u128 {
            let object = tiny_plan_program(
                "c:reg[4], r:reg[4]",
                &format!("c = lift.const.i4 {constant}\nr = lift.{op} x c\nlift.retval r"),
                "x:wire[4] in",
                "",
            );
            let plan = analyze(
                &object,
                "tiny",
                budget(3, 0, 0),
                &[],
                &[],
                AnalysisOrder::Forward,
            )
            .unwrap();
            for x in 0..16u128 {
                let value = match op {
                    "add" => x + constant,
                    "sub" => x.wrapping_sub(constant),
                    "mul" => x * constant,
                    "and" => x & constant,
                    "or" => x | constant,
                    "xor" => x ^ constant,
                    "shl" if constant < 4 => x << constant,
                    "lshr" if constant < 4 => x >> constant,
                    "ashr" if constant < 4 => ((((x as i8) << 4) >> 4) >> constant) as u128,
                    _ => 0,
                } & 15;
                assert!(
                    plan.node(2, &[]).unwrap().operands[0].contains(value),
                    "{op} x={x} c={constant}"
                );
            }
        }
    }
    for (op, expected) in [
        ("mul", 9),
        ("add", 6),
        ("sub", 0),
        ("ashr", 0),
        ("lshr", 0),
        ("shl", 8),
    ] {
        let object = tiny_plan_program(
            "a:reg[4], r:reg[4]",
            &format!("a = lift.const.i4 3\nr = lift.{op} a a\nlift.retval r"),
            "",
            "",
        );
        let plan = analyze(
            &object,
            "tiny",
            budget(3, 0, 0),
            &[],
            &[],
            AnalysisOrder::Forward,
        )
        .unwrap();
        assert_eq!(
            plan.node(2, &[]).unwrap().operands[0].as_u128(),
            Some(expected)
        );
    }
}

#[test]
fn analyzed_plan_preserves_first_fault_and_original_cutoffs() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    let object = tiny_plan_program(
        "c:reg[4]",
        "c = lift.const.i4 7\nlift.fault 3\n^dead():\nlift.fault 5\n^done():\nlift.retval c",
        "",
        "",
    );
    for steps in 0..=4 {
        let plan = analyze(
            &object,
            "tiny",
            budget(steps, 0, 0),
            &[],
            &[],
            AnalysisOrder::Forward,
        )
        .unwrap();
        assert_eq!(plan.node(0, &[]).unwrap().source_charge, 1);
        assert!(plan.node(2, &[]).is_none());
        assert_eq!(
            plan.node(1, &[]).unwrap().successors[0].fault,
            Some(Some(3))
        );
        for key in plan.layer(steps) {
            assert_eq!(
                plan.boundary_fault(key),
                if steps < 2 { Some(1) } else { None }
            );
        }
    }
    let object = tiny_plan_program(
        "c:reg[4], r:reg[4]",
        "c = lift.const.i4 0\nr = lift.udiv x c\nlift.retval r",
        "x:wire[4] in",
        "",
    );
    let plan = analyze(
        &object,
        "tiny",
        budget(4, 0, 0),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    assert_eq!(plan.node(1, &[]).unwrap().successors.len(), 1);
    assert_eq!(
        plan.node(1, &[]).unwrap().successors[0].fault,
        Some(Some(5))
    );
    assert!(plan.node(2, &[]).is_none());
}

#[test]
fn analyzed_plan_binding_captures_classical_args_controls_and_source_identity() {
    use bennett_circuits::{
        ir::Ctrl,
        oracle::{Argument, Budget},
        oracle_plan::{AnalysisOrder, analyze},
    };
    let object = tiny_plan_program(
        "r:reg[4]",
        "r = lift.trunc.i4 arg\nlift.retval r",
        "",
        "arg:i8",
    );
    let controls = [Ctrl {
        qubit: QubitRef::new("outside", 7),
        positive: false,
    }];
    let args = [Argument::Integer("23".into())];
    let plan = analyze(
        &object,
        "tiny",
        Budget {
            steps: 2,
            heap: 0,
            stack: 0,
            depth: 0,
        },
        &args,
        &controls,
        AnalysisOrder::Forward,
    )
    .unwrap();
    assert_eq!(plan.binding().arguments, args);
    assert_eq!(plan.binding().inherited_controls, controls);
    assert_eq!(
        plan.binding().closure_hash,
        object.check().unwrap().symbols["tiny"]
            .closure_hash
            .clone()
            .unwrap()
    );
    assert_eq!(plan.node(1, &[]).unwrap().operands[0].as_u128(), Some(7));
    assert!(
        analyze(
            &object,
            "tiny",
            budget(2, 0, 0),
            &[],
            &[],
            AnalysisOrder::Forward
        )
        .is_err()
    );
    let other = analyze(
        &object,
        "tiny",
        budget(2, 0, 0),
        &[Argument::Integer("24".into())],
        &controls,
        AnalysisOrder::Forward,
    )
    .unwrap();
    assert_ne!(plan.binding(), other.binding());
}

#[test]
fn analyzed_plan_reports_compact_work_without_claiming_emitted_gate_reduction() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    let object = checked_collatz();
    let plan = analyze(
        &object,
        "ref",
        budget(u64::MAX, 32, 32),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    let descriptors = plan.layers().descriptor_count();
    assert!(descriptors < 256);
    assert!(plan.analysis_visits < 4096);
    for steps in [0, 1, 2, 4, 8, 9, 16, 32, 64, 2048, u64::MAX] {
        let expanded = plan
            .layers()
            .expanded_row_contexts(steps, plan.rows().len());
        assert!(expanded <= u128::from(steps) * 33);
        eprintln!(
            "B={steps} analyzed_row_contexts={expanded} resident_layer_descriptors={descriptors} analysis_visits={}",
            plan.analysis_visits
        );
    }
    let legacy_program = legacy(&object, budget(9, 32, 32));
    assert_eq!(forward_counts(&legacy_program)[8], 238_029);
    assert_eq!(legacy_program.top_circuit().unwrap().body.len(), 884_677);
    eprintln!("legacy emitted gates unchanged: 238029/forward step, 476058/full marginal step");
}

#[test]
fn analyzed_plan_small_calls_match_legacy_and_pinned_reference() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    let object = plan_object(include_str!("fixtures/export_oracle/plan/calls.bennett"));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/oracle-plan-reference");
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = dir.join("calls.bennett");
    std::fs::write(&fixture, object.text()).unwrap();
    let binary = sibling::isa().join("reference-interpreter/target/debug/bennett-isa-ref");
    for stack in [5, 6, 11, 12] {
        for depth in [0, 1] {
            for steps in [0, 1, 4, 5, 8, 9] {
                let b = budget(steps, stack, depth);
                let plan = analyze(&object, "calls", b, &[], &[], AnalysisOrder::Forward).unwrap();
                let literal =
                    Permutation::from_literal(specialise_legacy(&object, "calls", b, &[]).unwrap());
                literal.check_xor_outputs(0..6).unwrap();
                let expected = |x: u64| {
                    if steps == 0 {
                        (0, 1, 0)
                    } else if stack < 6 || depth == 0 {
                        (0, 3, 1)
                    } else if steps >= 5 && stack < 12 {
                        (0, 3, 5)
                    } else if steps >= 9 {
                        ((x + 2) & 3, 0, 9)
                    } else {
                        (0, 1, steps)
                    }
                };
                let tuples = (0..256u64)
                    .map(|input| {
                        let (payload, code, _) = expected(input >> 6);
                        (
                            input,
                            (input & 3) ^ payload
                                | ((((input >> 2) & 15) ^ code) << 2)
                                | ((input >> 6) << 6),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_tuples(&literal, &tuples);
                for x in 0..4u64 {
                    let out = Command::new(&binary)
                        .args([
                            "run-oracle",
                            fixture.to_str().unwrap(),
                            "calls",
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
                    let value = Value::parse(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
                    let (payload, code, executed) = expected(x);
                    for (name, want) in [("payload", payload), ("code", code), ("steps", executed)]
                    {
                        assert_eq!(value.get(name).and_then(Value::as_u64), Some(want));
                    }
                    if code == 3 && executed == 1 {
                        assert!(
                            plan.node(0, &[])
                                .unwrap()
                                .successors
                                .iter()
                                .all(|e| e.target.is_none())
                        );
                    }
                    if code == 3 && executed == 5 {
                        assert!(
                            plan.node(1, &[])
                                .unwrap()
                                .successors
                                .iter()
                                .all(|e| e.target.is_none())
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn analyzed_plan_counter_widening_and_unknown_indirect_edges_are_sound() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, Check, Event, analyze};
    let source=include_str!("fixtures/export_oracle/plan/calls.bennett")
        .replace("a:reg[2], b:reg[2], saved:reg[2]","a:reg[2], b:reg[2], saved:reg[2], p:reg[1]")
        .replace("a = lift.call @inc (x)\nb = lift.call @inc (a)\nlift.retval b","p = lift.trunc.i1 x\nqjump ^loop\n^loop():\na = lift.call @inc (x)\nqbranch p ^loop ^done\n^done():\nlift.retval a")
        .replace("\"S_cells\":4","\"S_cells\":5");
    // This is a new source/layout variant, so derive its own identities rather
    // than carrying the frozen fixture's interface hash across a layout change.
    let (_, functions) = source.split_once("\n\nsection functions").unwrap();
    let variant = format!(
        "{}\n\nsection symbols\n{{}}\n\nsection functions{}",
        source.lines().next().unwrap(),
        functions
    );
    let object = plan_object(&variant);
    let plan = analyze(
        &object,
        "calls",
        budget(2048, 18, 1),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    assert!(
        matches!(plan.node(2,&[]).unwrap().event, Event::Call { stack:s, depth:Some(0), check:Check::Dynamic,.. } if s.min==0 && s.max==18)
    );
    assert!(
        plan.node(2, &[])
            .unwrap()
            .successors
            .iter()
            .any(|e| e.fault == Some(Some(3)))
    );
    for order in [
        AnalysisOrder::FunctionsReversed,
        AnalysisOrder::BlocksReversed,
    ] {
        let other = analyze(&object, "calls", budget(2048, 18, 1), &[], &[], order).unwrap();
        assert_eq!(plan.nodes(), other.nodes());
        assert_eq!(plan.layers(), other.layers());
    }
    let indirect = tiny_plan_program(
        "c:reg[4]",
        "qjumpr address\n^done():\nc = lift.const.i4 3\nlift.retval c",
        "address:wire[32] in",
        "",
    );
    let plan = analyze(
        &indirect,
        "tiny",
        budget(4, 0, 0),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    assert_eq!(
        plan.node(0, &[])
            .unwrap()
            .successors
            .iter()
            .filter(|e| e.target.is_some())
            .count(),
        3
    );
    assert!(
        plan.node(0, &[])
            .unwrap()
            .successors
            .iter()
            .any(|e| e.fault == Some(Some(4)))
    );
}

#[test]
fn analyzed_plan_unproved_memory_uses_general_kernels_and_forgets_writes() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    let source = "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{}\n\nsection functions\noracle @memory () (t:wire[8]) (f:wire[4]) () ()\nregs { p:reg[32], r:reg[8] }\n^entry():\np = lift.const.i32 0\nr = lift.load.i8 p\nlift.retval r\n\nsection layout\n{\"A_tab\":0,\"H\":12,\"S_cells\":0,\"globals\":{},\"heap_base\":4096,\"heap_limit_max\":0,\"interface_regions\":{\"memory\":{}},\"null_size\":4096,\"ptr_width\":32}\n";
    let object = plan_object(source);
    let plan = analyze(
        &object,
        "memory",
        budget(3, 0, 0),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    assert!(plan.node(1, &[]).unwrap().general_kernel);
    assert!(
        plan.node(1, &[])
            .unwrap()
            .successors
            .iter()
            .any(|e| e.target.is_none())
    );
    assert!(
        plan.node(2, &[]).unwrap().operands[0]
            .0
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn analyzed_plan_signed_comparisons_casts_and_parallel_aliases_are_exact() {
    use bennett_circuits::oracle_plan::{AnalysisOrder, analyze};
    for (predicate, expected) in [
        ("eq", 0),
        ("ne", 1),
        ("slt", 1),
        ("sle", 1),
        ("ult", 1),
        ("ule", 1),
    ] {
        let object = tiny_plan_program(
            "a:reg[4], b:reg[4], p:reg[1], r:reg[4]",
            &format!(
                "a = lift.const.i4 -8\nb = lift.const.i4 -1\np = lift.icmp.{predicate} a b\nr = lift.zext.i4 p\nlift.retval r"
            ),
            "",
            "",
        );
        let plan = analyze(
            &object,
            "tiny",
            budget(5, 0, 0),
            &[],
            &[],
            AnalysisOrder::Forward,
        )
        .unwrap();
        assert_eq!(
            plan.node(4, &[]).unwrap().operands[0].as_u128(),
            Some(expected)
        );
    }
    for (cast, expected) in [("sext", 255), ("zext", 15)] {
        let object = tiny_plan_program(
            "a:reg[4], b:reg[8], r:reg[4]",
            &format!(
                "a = lift.const.i4 -1\nb = lift.{cast}.i8 a\nr = lift.trunc.i4 b\nlift.retval r"
            ),
            "",
            "",
        );
        let plan = analyze(
            &object,
            "tiny",
            budget(4, 0, 0),
            &[],
            &[],
            AnalysisOrder::Forward,
        )
        .unwrap();
        assert_eq!(
            plan.node(2, &[]).unwrap().operands[0].as_u128(),
            Some(expected)
        );
    }
    let object = tiny_plan_program(
        "a:reg[4], b:reg[4]",
        "a = lift.const.i4 1\nb = lift.const.i4 2\n(a, b) = lift.parmov (b, a)\nlift.retval a",
        "",
        "",
    );
    let plan = analyze(
        &object,
        "tiny",
        budget(4, 0, 0),
        &[],
        &[],
        AnalysisOrder::Forward,
    )
    .unwrap();
    let b = plan.locations().iter().position(|l| l.name == "b").unwrap();
    assert_eq!(plan.node(3, &[]).unwrap().operands[0].as_u128(), Some(2));
    assert_eq!(plan.node(3, &[]).unwrap().values[b].as_u128(), Some(1));
}

#[test]
fn engineered_collatz_one_step_meets_actual_gate_envelope() {
    let object = checked_collatz();
    let program =
        bennett_circuits::oracle::specialise(&object, "ref", budget(1, 32, 32), &[]).unwrap();
    let count = program.top_circuit().unwrap().body.len();
    assert!(count <= 3000, "actual B1 gate nodes {count} exceed 3000");
}

#[test]
fn engineered_collatz_small_budget_matches_legacy_with_complete_cleanup() {
    use bennett_circuits::oracle_emit::prepare;
    let object = checked_collatz();
    for steps in [0, 1, 2, 4, 8, 9] {
        let b = budget(steps, 32, 32);
        let recipes = prepare(&object, "ref", b, &[]).unwrap().unwrap();
        let counts = recipes.counts();
        eprintln!(
            "B={steps} forward={} commit={} full={} qubits={} peak_ops={}",
            counts.forward, counts.commit, counts.full, counts.qubits, counts.peak_gate_objects
        );
        assert!(counts.full <= u128::from(steps) * 2000 + 1000);
        let executable = Permutation::from_literal(recipes.to_program().unwrap());
        executable.check_xor_outputs(0..12).unwrap();
        assert_tuples(&executable, &collatz_tuples(b));
        let baseline = Permutation::from_literal(legacy(&object, b));
        baseline.check_xor_outputs(0..12).unwrap();
        assert_tuples(&baseline, &collatz_tuples(b));
    }
}

#[test]
fn engineered_small_oracle_exhausts_full_abi_from_executable_qasm() {
    use bennett_circuits::oracle_emit::prepare;
    let object = Object::parse(include_str!("fixtures/export_oracle/increment2.bennett")).unwrap();
    for steps in 0..=12 {
        let recipes = prepare(&object, "inc2", budget(steps, 0, 0), &[])
            .unwrap()
            .unwrap();
        let text = qasm::export(&recipes.to_program().unwrap()).unwrap();
        let text = text
            .lines()
            .filter(|line| !line.starts_with("// bc:"))
            .collect::<Vec<_>>()
            .join("\n");
        let executable = Permutation::from_flat(qasm::parse_flat(&text).unwrap()).unwrap();
        executable.check_xor_outputs(0..6).unwrap();
        let tuples = (0..256u64)
            .map(|input| {
                let (target, fault, x) = (input & 3, (input >> 2) & 15, input >> 6);
                let (payload, code) = if steps < 3 { (0, 1) } else { ((x + 1) & 3, 0) };
                (input, (target ^ payload) | ((fault ^ code) << 2) | (x << 6))
            })
            .collect::<Vec<_>>();
        assert_tuples(&executable, &tuples);
    }
}

#[test]
fn engineered_collatz_recipe_counts_meet_full_envelope_without_gate_arrays() {
    use bennett_circuits::oracle_emit::prepare;
    let object = checked_collatz();
    for steps in [0, 1, 2, 4, 8, 9, 16, 32, 64, 2048] {
        let recipes = prepare(&object, "ref", budget(steps, 32, 32), &[])
            .unwrap()
            .unwrap();
        let counts = recipes.counts();
        assert!(
            counts.full <= u128::from(steps) * 2000 + 1000,
            "B={steps} {} gates",
            counts.full
        );
        assert!(counts.peak_gate_objects <= 50_000);
        assert!(counts.layer_descriptors < 256);
        assert_eq!(counts.kernels.stack_scans, 0);
        assert_eq!(counts.kernels.generic_multiplications, 0);
        assert_eq!(counts.kernels.variable_shifts, 0);
        assert_eq!(counts.kernels.ordered_comparisons, 0);
        let mut visited = 0u128;
        let mut histogram = BTreeMap::new();
        recipes
            .visit(|gate| {
                visited += 1;
                *histogram.entry(gate.controls.len()).or_insert(0u128) += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(visited, counts.full);
        assert_eq!(
            histogram,
            counts
                .histogram
                .iter()
                .filter(|(_, v)| **v > 0)
                .map(|(k, v)| (*k, *v))
                .collect()
        );
        eprintln!(
            "ACTUAL B={steps} forward={} full={} workspace={} history={} peak_ops={} descriptors={} max_controls={}",
            counts.forward,
            counts.full,
            counts.qubits - recipes.interface_wires() as u128,
            counts.history_wires,
            counts.peak_gate_objects,
            counts.layer_descriptors,
            counts.max_controls
        );
        if steps == 2048 {
            let period = (28..46)
                .map(|t| recipes.forward_gates_at(t))
                .collect::<Vec<_>>();
            eprintln!("actual G_c at B2048: {:?}", counts.histogram);
            eprintln!(
                "steady forward period={period:?}, max={}, sum={}",
                period.iter().max().unwrap(),
                period.iter().sum::<usize>()
            );
        }
    }
}

/// Independent wire-major bit-sliced execution of visited gates, without a gate
/// array or product simulator. All ABI probes share one emitted traversal.
fn assert_recipe_tuples(recipes: &bennett_circuits::oracle_emit::Recipes, tuples: &[(u64, u64)]) {
    let wires = recipes.wire_count().unwrap();
    let words = tuples.len().div_ceil(64);
    let interface = recipes.interface_wires();
    let mut state = vec![0u64; wires * words];
    for (lane, (input, _)) in tuples.iter().enumerate() {
        for wire in 0..interface {
            state[wire * words + lane / 64] |= ((input >> wire) & 1) << (lane % 64);
        }
    }
    for invocation in 0..2 {
        let mut visited = 0u128;
        recipes
            .visit(|gate| {
                assert!(gate.target < wires);
                assert!(
                    gate.target < 12 || gate.target >= interface,
                    "read-only input write"
                );
                let mut used = BTreeSet::from([gate.target]);
                for (q, _) in &gate.controls {
                    assert!(*q < wires && used.insert(*q));
                    assert!(*q >= 12, "output used as a control");
                }
                for word in 0..words {
                    let enabled = gate.controls.iter().fold(u64::MAX, |mask, (q, positive)| {
                        mask & if *positive {
                            state[q * words + word]
                        } else {
                            !state[q * words + word]
                        }
                    });
                    state[gate.target * words + word] ^= enabled;
                }
                visited += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(visited, recipes.counts().full);
        for (lane, (input, output)) in tuples.iter().enumerate() {
            let expected = if invocation == 0 { *output } else { *input };
            for wire in 0..interface {
                assert_eq!(
                    (state[wire * words + lane / 64] >> (lane % 64)) & 1,
                    (expected >> wire) & 1,
                    "wire={wire} input={input} invocation={invocation}"
                );
            }
        }
        let last_mask = if tuples.len() % 64 == 0 {
            u64::MAX
        } else {
            (1u64 << (tuples.len() % 64)) - 1
        };
        for wire in interface..wires {
            for word in 0..words {
                let mask = if word + 1 == words {
                    last_mask
                } else {
                    u64::MAX
                };
                assert_eq!(
                    state[wire * words + word] & mask,
                    0,
                    "dirty private wire={wire} invocation={invocation}"
                );
            }
        }
    }
}

#[test]
fn engineered_collatz_large_boundaries_match_pinned_reference_for_all_inputs() {
    use bennett_circuits::oracle_emit::prepare;
    let object = checked_collatz();
    let binary = sibling::isa().join("reference-interpreter/target/debug/bennett-isa-ref");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/export_oracle/collatz.bennett");
    for steps in [10, 27, 28, 29, 45, 46, 47, 2048] {
        let recipes = prepare(&object, "ref", budget(steps, 32, 32), &[])
            .unwrap()
            .unwrap();
        let mut tuples = vec![];
        for x in 0..256u64 {
            let out = Command::new(&binary)
                .args([
                    "run-oracle",
                    fixture.to_str().unwrap(),
                    "ref",
                    "--input",
                    &x.to_string(),
                    "--budget",
                    &format!("{steps},0,32,32"),
                ])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let result = Value::parse(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
            let payload = result.get("payload").and_then(Value::as_u64).unwrap();
            let code = result.get("code").and_then(Value::as_u64).unwrap();
            for (target, fault) in collatz_probes() {
                tuples.push((
                    target | (fault << 8) | (x << 12),
                    (target ^ payload) | ((fault ^ code) << 8) | (x << 12),
                ));
            }
        }
        assert_recipe_tuples(&recipes, &tuples);
        eprintln!(
            "reference/recipe agreement B={steps}: 256 inputs x16 outputs; all private bits clean and involution"
        );
    }
}

#[test]
fn engineered_collatz_capacity_matrix_preserves_first_faults() {
    use bennett_circuits::oracle_emit::prepare;
    let object = checked_collatz();
    for (stack, depth) in CAPACITY_LIMITS {
        for steps in CAPACITY_STEPS {
            let b = budget(steps, stack, depth);
            let recipes = prepare(&object, "ref", b, &[]).unwrap().unwrap();
            assert_recipe_tuples(&recipes, &collatz_tuples(b));
        }
    }
}

fn small_recipe_object(regs: &str, body: &str, inputs: &str) -> Object {
    plan_object(&format!(
        "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\noracle @small ({inputs}) (t:wire[2]) (f:wire[4]) () ()\nregs {{ {regs} }}\n^entry():\n{body}\n"
    ))
}
fn assert_small_recipes(
    object: &Object,
    name: &str,
    b: Budget,
    expected: impl Fn(u64) -> (u64, u64),
) {
    use bennett_circuits::oracle_emit::prepare;
    let recipes = prepare(object, name, b, &[]).unwrap().unwrap();
    let program = recipes.to_program().unwrap();
    let history = program
        .top_circuit()
        .unwrap()
        .workspace
        .iter()
        .filter(|w| w.kind == bennett_circuits::ir::WsKind::RetainedHistory)
        .map(|w| u128::from(w.width))
        .sum::<u128>();
    assert_eq!(history, recipes.counts().history_wires);
    let executable = Permutation::from_literal(program);
    executable.check_xor_outputs(0..6).unwrap();
    let tuples = (0..1u64 << executable.interface)
        .map(|input| {
            let (payload, code) = expected(input >> 6);
            (
                input,
                ((input & 3) ^ payload) | ((((input >> 2) & 15) ^ code) << 2) | ((input >> 6) << 6),
            )
        })
        .collect::<Vec<_>>();
    assert_tuples(&executable, &tuples);
}

#[test]
fn engineered_small_kernels_exhaust_all_inputs_outputs_and_modular_boundaries() {
    for op in [
        "add", "sub", "mul", "and", "or", "xor", "shl", "lshr", "ashr",
    ] {
        let object = small_recipe_object(
            "r:reg[2]",
            &format!("r = lift.{op} x y\nlift.retval r"),
            "x:wire[2] in, y:wire[2] in",
        );
        assert_small_recipes(&object, "small", budget(2, 0, 0), |input| {
            let (x, y) = (input & 3, input >> 2);
            let value = match op {
                "add" => x + y,
                "sub" => x.wrapping_sub(y),
                "mul" => x * y,
                "and" => x & y,
                "or" => x | y,
                "xor" => x ^ y,
                "shl" if y < 2 => x << y,
                "lshr" if y < 2 => x >> y,
                "ashr" if y < 2 => ((((x as i8) << 6) >> 6) >> y) as u64,
                _ => 0,
            } & 3;
            (value, 0)
        });
    }
    for op in ["mul", "shl", "lshr", "ashr"] {
        for constant in 0..=3 {
            let object = small_recipe_object(
                "c:reg[2], r:reg[2]",
                &format!("c = lift.const.i2 {constant}\nr = lift.{op} x c\nlift.retval r"),
                "x:wire[2] in",
            );
            assert_small_recipes(&object, "small", budget(3, 0, 0), |x| {
                let value = match op {
                    "mul" => x * constant,
                    "shl" if constant < 2 => x << constant,
                    "lshr" if constant < 2 => x >> constant,
                    "ashr" if constant < 2 => ((((x as i8) << 6) >> 6) >> constant) as u64,
                    _ => 0,
                } & 3;
                (value, 0)
            });
        }
    }
    for pred in ["eq", "ne", "ult", "ule", "slt", "sle"] {
        let object = small_recipe_object(
            "p:reg[1], r:reg[2]",
            &format!("p = lift.icmp.{pred} x y\nr = lift.zext.i2 p\nlift.retval r"),
            "x:wire[2] in, y:wire[2] in",
        );
        assert_small_recipes(&object, "small", budget(3, 0, 0), |input| {
            let (x, y) = (input & 3, input >> 2);
            let (sx, sy) = ((((x as i8) << 6) >> 6), (((y as i8) << 6) >> 6));
            let predicate = match pred {
                "eq" => x == y,
                "ne" => x != y,
                "ult" => x < y,
                "ule" => x <= y,
                "slt" => sx < sy,
                _ => sx <= sy,
            };
            (u64::from(predicate), 0)
        });
    }
    let object = small_recipe_object(
        "a:reg[2], b:reg[2]",
        "a = lift.mov x\nb = lift.mov y\n(a, b) = lift.parmov (b, a)\nlift.retval a",
        "x:wire[2] in, y:wire[2] in",
    );
    assert_small_recipes(&object, "small", budget(4, 0, 0), |input| (input >> 2, 0));
    let object = small_recipe_object(
        "p:reg[1], r:reg[2]",
        "p = lift.trunc.i1 x\nr = lift.select p x y\nlift.retval r",
        "x:wire[2] in, y:wire[2] in",
    );
    assert_small_recipes(&object, "small", budget(3, 0, 0), |input| {
        let (x, y) = (input & 3, input >> 2);
        (if x & 1 != 0 { x } else { y }, 0)
    });
}

#[test]
fn engineered_calls_reinitialize_banks_and_preserve_monotonic_capacity() {
    let object = plan_object(include_str!("fixtures/export_oracle/plan/calls.bennett"));
    for stack in [5, 6, 11, 12] {
        for depth in [0, 1] {
            for steps in 0..=12 {
                assert_small_recipes(&object, "calls", budget(steps, stack, depth), |x| {
                    if steps == 0 {
                        (0, 1)
                    } else if stack < 6 || depth == 0 {
                        (0, 3)
                    } else if steps >= 5 && stack < 12 {
                        (0, 3)
                    } else if steps >= 9 {
                        ((x + 2) & 3, 0)
                    } else {
                        (0, 1)
                    }
                });
            }
        }
    }
    let object = plan_object(include_str!(
        "fixtures/export_oracle/plan/calls-loop.bennett"
    ));
    for stack in [0, 5, 6, 11, 12, 18] {
        for depth in [0, 1] {
            for steps in [0, 1, 2, 3, 6, 7, 8, 12, 13, 17, 18, 19, 32] {
                assert_small_recipes(&object, "loop", budget(steps, stack, depth), |x| {
                    let first_fault = if depth == 0 { 3 } else { 3 + 5 * (stack / 6) };
                    if steps >= 3 && (stack < 6 || depth == 0) {
                        (0, 3)
                    } else if x & 1 == 0 && steps >= 8 {
                        ((x + 1) & 3, 0)
                    } else if x & 1 != 0 && steps >= first_fault {
                        (0, 3)
                    } else {
                        (0, 1)
                    }
                });
            }
        }
    }
}

#[test]
fn engineered_diamonds_labels_partial_faults_and_classical_arguments_are_exact() {
    use bennett_circuits::{oracle::Argument, oracle_emit::prepare};
    let diamond = plan_object(include_str!("fixtures/export_oracle/plan/diamond.bennett"));
    for steps in 0..=9 {
        assert_small_recipes(&diamond, "diamond", budget(steps, 0, 0), |x| {
            if steps < 8 {
                (0, 1)
            } else {
                (x & if x & 1 != 0 { 1 } else { 3 }, 0)
            }
        });
    }
    let faults = small_recipe_object(
        "c:reg[2]",
        "c = lift.const.i2 0\nlift.faultif p 3\nlift.fault 5\n^done():\nlift.retval c",
        "p:wire[1] in",
    );
    for steps in 0..=4 {
        assert_small_recipes(&faults, "small", budget(steps, 0, 0), |p| {
            if p == 1 && steps >= 2 {
                (0, 3)
            } else if steps >= 3 {
                (0, 5)
            } else {
                (0, 1)
            }
        });
    }
    let object = tiny_plan_program(
        "r:reg[4]",
        "r = lift.trunc.i4 arg\nlift.retval r",
        "",
        "arg:i8",
    );
    let recipes = prepare(
        &object,
        "tiny",
        budget(2, 0, 0),
        &[Argument::Integer("23".into())],
    )
    .unwrap()
    .unwrap();
    let executable = Permutation::from_literal(recipes.to_program().unwrap());
    let tuples = (0..256u64)
        .map(|input| (input, ((input & 15) ^ 7) | (input & 240)))
        .collect::<Vec<_>>();
    assert_tuples(&executable, &tuples);
    let bad_label = small_recipe_object(
        "address:reg[32], r:reg[2]",
        "address = lift.const.i32 999\nqjumpr address\n^done():\nlift.retval r",
        " ",
    );
    assert_small_recipes(&bad_label, "small", budget(2, 0, 0), |_| (0, 4));
}

#[test]
fn engineered_positive_negative_controls_preserve_external_unknown_state() {
    use bennett_circuits::ir::{Bundle, Circuit, Ctrl, Role};
    let object = Object::parse(include_str!("fixtures/export_oracle/increment2.bennett")).unwrap();
    for positive in [false, true] {
        let mut program =
            bennett_circuits::oracle::specialise(&object, "inc2", budget(3, 0, 0), &[]).unwrap();
        let root = Circuit {
            name: "controlled".into(),
            perm: true,
            mut_bundles: vec![
                Bundle {
                    name: "t".into(),
                    width: 2,
                    role: Role::Mut,
                },
                Bundle {
                    name: "f".into(),
                    width: 4,
                    role: Role::Mut,
                },
            ],
            in_bundles: vec![
                Bundle {
                    name: "x".into(),
                    width: 2,
                    role: Role::In,
                },
                Bundle {
                    name: "control".into(),
                    width: 1,
                    role: Role::In,
                },
                Bundle {
                    name: "outside".into(),
                    width: 1,
                    role: Role::In,
                },
            ],
            mem: vec![],
            workspace: vec![],
            local_regions: vec![],
            body: vec![Node::Call {
                callee: "inc2".into(),
                adjoint: false,
                controls: vec![Ctrl {
                    qubit: QubitRef::new("control", 0),
                    positive,
                }],
                args: vec!["t".into(), "f".into(), "x".into()],
            }],
        };
        program.circuits.insert(0, root);
        program.top = "controlled".into();
        let text = qasm::export(&program).unwrap();
        let executable = text
            .lines()
            .filter(|line| !line.starts_with("// bc:"))
            .collect::<Vec<_>>()
            .join("\n");
        let circuit = Permutation::from_flat(qasm::parse_flat(&executable).unwrap()).unwrap();
        circuit.check_xor_outputs(0..6).unwrap();
        let tuples = (0..1024u64)
            .map(|input| {
                let active = ((input >> 8) & 1 != 0) == positive;
                let payload = (((input >> 6) & 3) + 1) & 3;
                (input, if active { input ^ payload } else { input })
            })
            .collect::<Vec<_>>();
        assert_tuples(&circuit, &tuples);
    }
}

#[test]
fn engineered_fallbacks_preserve_complete_legacy_abis_and_errors() {
    use bennett_circuits::{oracle::specialise, oracle_emit::prepare};
    let recursive = plan_object(include_str!(
        "fixtures/export_oracle/plan/recursive.bennett"
    ));
    assert!(
        prepare(&recursive, "recurse", budget(2, 16, 16), &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        specialise(&recursive, "recurse", budget(2, 16, 16), &[]).unwrap(),
        specialise_legacy(&recursive, "recurse", budget(2, 16, 16), &[]).unwrap()
    );
    let divide = small_recipe_object(
        "r:reg[2]",
        "r = lift.sdiv x y\nlift.retval r",
        "x:wire[2] in, y:wire[2] in",
    );
    assert!(
        prepare(&divide, "small", budget(2, 0, 0), &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        specialise(&divide, "small", budget(2, 0, 0), &[]).unwrap(),
        specialise_legacy(&divide, "small", budget(2, 0, 0), &[]).unwrap()
    );
    let memory = Object::parse(
        &std::fs::read_to_string(
            sibling::isa().join("reference-interpreter/tests/fixtures/tiny_memory.bisa"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        prepare(&memory, "tiny", budget(0, 0, 0), &[])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        specialise(&memory, "tiny", budget(0, 0, 0), &[]).unwrap(),
        specialise_legacy(&memory, "tiny", budget(0, 0, 0), &[]).unwrap()
    );
    let ordinary =
        Object::parse(include_str!("fixtures/export_oracle/increment2.bennett")).unwrap();
    for limits in [
        Budget {
            steps: 0,
            heap: 1,
            stack: 0,
            depth: 0,
        },
        budget(0, 1, 0),
        budget(0, 0, 1),
    ] {
        assert!(prepare(&ordinary, "inc2", limits, &[]).is_err());
        assert!(specialise_legacy(&ordinary, "inc2", limits, &[]).is_err());
    }
    let recipes = prepare(&ordinary, "inc2", budget(3, 0, 0), &[])
        .unwrap()
        .unwrap();
    let mut seen = 0;
    let failure = recipes
        .visit(|_| {
            seen += 1;
            if seen == 7 {
                Err(bennett_circuits::Error("injected visitor error".into()))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(seen, 7);
    assert!(failure.0.contains("injected visitor error"));
}

#[test]
fn engineered_repeated_context_and_nested_calls_preserve_captured_arguments() {
    let object = plan_object(include_str!(
        "fixtures/export_oracle/plan/calls-until-zero.bennett"
    ));
    for stack in [6, 12, 18, 24] {
        for steps in [0, 3, 4, 9, 10, 15, 16, 21, 22, 27, 28, 32] {
            assert_small_recipes(&object, "untilzero", budget(steps, stack, 1), |x| {
                let calls = 4 - x;
                let success = 4 + 6 * calls;
                let available = stack / 6;
                if available < calls && steps >= 4 + 6 * available {
                    (0, 3)
                } else if available >= calls && steps >= success {
                    (0, 0)
                } else {
                    (0, 1)
                }
            });
        }
    }
    let nested = plan_object(include_str!(
        "fixtures/export_oracle/plan/calls-nested.bennett"
    ));
    for stack in [5, 6, 11, 12] {
        for depth in [0, 1, 2] {
            for steps in 0..=8 {
                assert_small_recipes(&nested, "nested", budget(steps, stack, depth), |x| {
                    if steps >= 1 && (stack < 6 || depth == 0) {
                        (0, 3)
                    } else if steps >= 2 && (stack < 12 || depth < 2) {
                        (0, 3)
                    } else if steps >= 7 {
                        ((x + 1) & 3, 0)
                    } else {
                        (0, 1)
                    }
                });
            }
        }
    }
}

#[test]
fn engineered_call_loop_fault_steps_are_adjudicated_by_pinned_reference() {
    let object = plan_object(include_str!(
        "fixtures/export_oracle/plan/calls-loop.bennett"
    ));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp/recipe-reference");
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = dir.join("loop.bennett");
    std::fs::write(&fixture, object.text()).unwrap();
    let binary = sibling::isa().join("reference-interpreter/target/debug/bennett-isa-ref");
    for (x, b, expected) in [
        (1, budget(2, 18, 1), (0, 1, 2)),
        (1, budget(17, 18, 1), (0, 1, 17)),
        (1, budget(18, 18, 1), (0, 3, 18)),
        (1, budget(19, 18, 1), (0, 3, 18)),
        (1, budget(8, 6, 1), (0, 3, 8)),
        (3, budget(13, 12, 1), (0, 3, 13)),
        (0, budget(8, 18, 1), (1, 0, 8)),
        (2, budget(8, 18, 1), (3, 0, 8)),
        (0, budget(3, 18, 0), (0, 3, 3)),
    ] {
        let output = Command::new(&binary)
            .args([
                "run-oracle",
                fixture.to_str().unwrap(),
                "loop",
                "--input",
                &x.to_string(),
                "--budget",
                &format!("{},0,{},{}", b.steps, b.stack, b.depth),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = Value::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
        for (field, want) in [
            ("payload", expected.0),
            ("code", expected.1),
            ("steps", expected.2),
        ] {
            assert_eq!(value.get(field).and_then(Value::as_u64), Some(want));
        }
    }
}

#[test]
fn engineered_collatz_cli_small_budgets_execute_without_ir_metadata() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/export_oracle/collatz.bennett");
    for steps in [1, 9] {
        let output = Command::new(env!("CARGO_BIN_EXE_bennett-circuits"))
            .args([
                "export-qasm",
                fixture.to_str().unwrap(),
                "ref",
                "--budget",
                &format!("{steps},0,32,32"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let executable = text
            .lines()
            .filter(|line| !line.starts_with("// bc:"))
            .collect::<Vec<_>>()
            .join("\n");
        let circuit = Permutation::from_flat(qasm::parse_flat(&executable).unwrap()).unwrap();
        circuit.check_xor_outputs(0..12).unwrap();
        assert_tuples(&circuit, &collatz_tuples(budget(steps, 32, 32)));
    }
}

#[test]
fn engineered_dispatch_keeps_nested_literal_oracles_independent() {
    use bennett_circuits::oracle::{Argument, specialise};
    let source = "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=32\n\nsection symbols\n{}\n\nsection functions\noracle @outer (x:wire[2] in) (t:wire[2]) (f:wire[4]) () (tk:token)\nregs { r:reg[2], code:reg[4] }\n^entry():\nlift.oracle_call @inner tk (x) r code () ()\nlift.retval r\noracle @inner (x:wire[2] in) (t:wire[2]) (f:wire[4]) () ()\nregs { one:reg[2], r:reg[2] }\n^entry():\none = lift.const.i2 1\nr = lift.add x one\nlift.retval r\n";
    let object = plan_object(source);
    let inner_budget = budget(3, 0, 0);
    let literal_count = specialise_legacy(&object, "inner", inner_budget, &[])
        .unwrap()
        .top_circuit()
        .unwrap()
        .body
        .len();
    let optimized_count = specialise(&object, "inner", inner_budget, &[])
        .unwrap()
        .top_circuit()
        .unwrap()
        .body
        .len();
    assert_ne!(
        literal_count, optimized_count,
        "witness needs different constructions"
    );
    let outer = specialise_legacy(
        &object,
        "outer",
        budget(1, 0, 0),
        &[Argument::Token(inner_budget)],
    )
    .unwrap();
    let nested = outer
        .circuits
        .iter()
        .find(|c| c.name.ends_with("inner"))
        .unwrap();
    assert_eq!(
        nested.body.len(),
        literal_count,
        "legacy nested call used the optimizer"
    );
    assert!(specialise_legacy(&object, "missing", inner_budget, &[]).is_err());
    assert_eq!(
        specialise(&object, "inner", inner_budget, &[])
            .unwrap()
            .top_circuit()
            .unwrap()
            .body
            .len(),
        optimized_count,
        "literal scope leaked past return/error"
    );
}

#[test]
fn engineered_recipe_resident_shape_is_independent_of_budget_doubling() {
    use bennett_circuits::oracle_emit::prepare;
    let object = checked_collatz();
    let first = prepare(&object, "ref", budget(2048, 32, 32), &[])
        .unwrap()
        .unwrap();
    let second = prepare(&object, "ref", budget(4096, 32, 32), &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        first.counts().layer_descriptors,
        second.counts().layer_descriptors
    );
    assert_eq!(
        first.counts().peak_gate_objects,
        second.counts().peak_gate_objects
    );
    assert!(second.counts().history_wires > first.counts().history_wires);
    assert!(second.counts().full > first.counts().full);
}
