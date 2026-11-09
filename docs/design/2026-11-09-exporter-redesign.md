# An engineered bounded-oracle exporter

Status: proposed, ready for the oracle/contract council (sol, astra, Muse).
Implementation must wait for council disposition. This document implements no
exporter change. Analysis baseline: circuits `5333b918c1cc3b8399014d6d727ffcaff2148b16`.

## Decision requested

Keep the literal bounded machine as an independent small-budget differential
oracle. Build an engineered, step-unrolled lowering behind the same validated
`export-qasm` entry point: specialize static call contexts, propagate constants,
emit only rows reachable at each original budget position, retain reversible
history explicitly, and stream executable output. Then add structured loop
specialization as a separately validated optimization. Neither changes F_B,
reference-object identities, the budget unit, fault priority, or the input domain.

The Collatz case supports a first acceptance envelope of **2,000 B + 1,000
logical gates**, including computation, commit and reversal: at B=2048,
4,097,000 gates. This is a proposed engineering requirement, not a measured
implementation result. A conservative source-based step model predicts 992,159;
fixed-shift aliasing improves that model to 819,335. A loop model predicts about
377,000. These estimates have different confidence from the exact current-code
counts below; no replacement circuit has been built or checked yet.

## 1. Contract and meaning of “the circuit for an oracle”

### Sources and versions

The actual `gate.toml` pins bennett-isa-spec
`ecaaac060c44e317d1181345fa6b8e38c096c099`. Its `spec/Bennett-ISA-Spec.md`,
ISA **0.8**, is normative here: §§3.6–3.7 (machine and call records),
§4.2 (reference body, rows and reindexing), §5.2 (oracle), §5.4 (budgets),
§5.5 (nested cleanup), §7.2 (layout), §9 (identity), §14 (reference interpreter),
and §15 T2/T6 (cleanup/agreement). `SPEC_VERSIONS.toml` also says 0.8;
explicit draft-frame opt-in does not silently migrate this object to 0.9.

For the producer's row meaning, the published dialect **0.2.14**, lifting map
**0.2.9**, `spec/Bennett-Dialect-Spec.md` §§9.4 and 16.1–16.5, was read at
`687284fa28a79c536b3885cabec3adf5de26e031`. Circuits has no dialect sibling pin;
this hash freezes the supplementary text consulted, not a new build dependency.
The fixture identifies ISA/object 0.8; it has no separate mapping-version
attestation. Its supplied bytes fix the reference rows for this design.

Today `src/main.rs::export_region` loads the object, checks stored identities
and source obligations, fixes classical arguments and budget, then calls
`oracle::specialise`. `src/oracle.rs` constructs B reversible forward source-row
steps, the commit, and the reverse of the complete forward prefix.
`src/qasm.rs::export` serializes this into OpenQASM 3 with hierarchical gate
definitions, explicit auxiliary wires, and a full `bc` IR comment. It retains
the program, builds metadata, and constructs an output String. This is a
circuit for bounded F_B, not the unbounded mathematical Collatz function.

For every legal budget and every interface basis tuple x in D:

- Inputs and input-memory cells are unchanged. Target is XORed with payload;
  fault is XORed with code, including arbitrary nonzero initial target/fault.
- Success means the **original** interpreter reaches `lift.retval` within B
  rows without fault. Otherwise payload is zero and code is the first fault,
  or code 1 if not halted at the boundary. B=0 is legal.
- Private workspace and locations return clean, local images are restored,
  and borrowed state is returned independently of entanglement with outsiders.
  There is no measurement or residual phase. The oracle is an involution.

ISA §3.7 already permits extensional inlining. Removing a physical stack
access or constant register does **not** remove its reference-row charge.
Canonical numeric labels observable through `lift.label` survive any internal
reindexing (§3.6). Hierarchy, private history and QASM bytes may change.
Source proof material, optimized execution equivalence and target realization
proof remain separate: do not reuse a source certificate as an unchanged
certificate of a new realization.

### Units

`G_c` counts logical X gates with c controls after call expansion; `G = Σ G_c`.
Negative controls count as controls. A 4-control X is one logical gate,
**not** one Toffoli. Report G_c, maximum controls, quantum workspace wires,
executable statements/bytes, hierarchy nodes and expanded gates separately.
No numerical gate claim here is a hardware T count.

For this object the current program is one circuit containing only
`Node::Gate`, so logical gates equal IR body nodes exactly. A marginal full
oracle count includes the added forward step **and** its inverse. An executed
source row is another unit; B is not a Collatz iteration count. Shared gate
definitions and QASM `for` syntax cannot reduce expanded-gate counts.

## 2. Current size: code-derived remeasurement

The supplied 54-line object (Appendix A) contains **33 executable rows**:
two wrapper rows and 31 lproc rows. HALT=33, pc width=6. There are 26 locations
overall, 25 in L1, with 907 lproc location bits. The call record is **28 cells**
(3+25), cell width=64. Counts below use `(B,0,32,32)`.

The ephemeral object and every named original measurement were copied first
to `.tmp/circuits-8xv/evidence/`. Object SHA-256:
`bd1ca28a40bc836625effc2121a89cfbfe3504463a4774f4a6006eee806a336c`.
The preserved allocator-failure backtrace reaches `Build::flip`, `copy`,
`exchange`, `array_exchange`, `row`, `forward`. It identifies a construction
path, not a gate-count receipt. Historical elapsed measurements and the
approximate memory/step claim are not used as acceptance evidence.

### Method and exactness boundary

Scratch `count.rs` extracts `Bit`, `Build`, relevant `Machine` methods and
original row arms from baseline `src/oracle.rs`. Only `Build::flip`'s gate
allocation is replaced with a control-count entry. Literal/alias/contradictory
control simplifications, xor/and/mux, arithmetic, copy and exchange routines
remain identical. It instantiates this object's locations, 32 stack cells
and counters, counts each row and successor, then applies the unchanged
`possible_successors` recurrence. Unused memory/nested-action arms are omitted;
an unused address read explicitly errors. This is a **source-cost counter**,
not a semantic interpreter or completed end-to-end export.

Commands, with headless environment and job-local TMPDIR:

```sh
rustc --edition=2024 -Awarnings .tmp/circuits-8xv/count.rs -o .tmp/circuits-8xv/count
.tmp/circuits-8xv/count > .tmp/circuits-8xv/evidence/counts.txt
```

The finite counter does not materialize a B-step circuit. No repository
hang-guard class table was found; this uses the brief's alternative of counting
from lowering code. No long exporter run, full gate or conformance run was
performed for this design. Scratch harnesses, commands and receipts remain
local; equations and the full fixture below make the argument reviewable
without publishing operational logs.

Let S_t be the possible pc set at step t, initially `{0}`; r_k includes row k
and its successor XOR, before dispatch. Exactly for this source/object family:

```
S_(t+1) = union(possible_successors(k), k in S_t)
f_t = 2 |S_t| + 6 P + sum(r_k, k in S_t excluding HALT)
      + popcount(HALT) [HALT in S_t]
G(B) = N_IR(B) = 2 sum(f_t, t=1..B) + 15
```

The 15 commit gates are two halt comparators, eight target XORs, four code
XORs and one step-fault constant XOR. Initial layout values are zero here
and add no gates. Six-P counts two pc exchanges, each three XORs per bit.

| B | Forward gates in step B | Full-oracle gates = IR nodes |
|---:|---:|---:|
| 0 | — | 15 |
| 1 | 92,331 | 184,677 |
| 2 | 71 | 184,819 |
| 4 | 56 | 185,153 |
| 8 | 111,006 | 408,619 |
| 9 | 238,029 | 884,677 |
| 16 | 238,029 | 4,217,083 |
| 32 | 238,029 | 11,834,011 |
| 2048 | 238,029 | 971,566,939 |

Large-budget entries are exact **code-derived projections**, not completed
exports. After step 8, all rows and HALT occur in S_t. For B≥8,
`G(B) = 408619 + 476058 (B-8)`. Each additional budget step adds **476,058
IR gate nodes**, although the real input executes at most one source row.

### Three proposed causes, corrected

1. **Stack machinery: confirmed, units corrected.** The call row contributes
   92,290 gates, return 110,951; their successor XORs add 3 and 6.
   The two 907-bit location exchanges scan 32 cells: those scans alone use
   `2 × 907 × 32 × 3 = 174144` controlled X gates per full forward layer.
   Call+successor has histogram `{G1:240,G2:4669,G3:120,G4:87264}`;
   return+successor has `{G1:12150,G2:11439,G3:56,G4:87312}`.
   Together that is **203,250 logical gates** forward, 406,500 with inverse,
   mostly 4-control Xs; it is not the bead's ~150,000 Toffolis/step.
   The dynamic call depth is one, but generic quantum-stack scans remain.
   Their insufficient-capacity outcomes must survive replacing the scans.
2. **Materialized constant arithmetic: confirmed, magnitude corrected.**
   `lift.const` writes a quantum location; `read` returns wires rather than
   `Bit::C`. Multiply by 3 therefore uses 64 partial rows and 64 adders:
   15,940 kernel gates; 16,197 with assignment/fault bookkeeping, plus 4
   successor gates. Lshr has 13,179 kernel gates, or 13,436+4 for its row.
   Its 64 selector stages have distances 1,2,4,..., not 64 shifts by one;
   high stages and the large-shift guard implement shift≥64→0. The two
   kernels total 29,119 per forward layer, less than the stack scans.
   A literal 3 reduces the existing mul kernel to 437. Literal lshr still
   emits 758: `mux` eagerly builds xor intermediates before selecting a
   constant arm. A fixed shift should be an alias map with **zero kernel
   gates**, plus whatever output/update copies are necessary.
3. **Return successor explosion: confirmed, onset corrected.**
   `possible_successors("ret") = 0..=n`. The shortest input path reaches
   return in step 8; step 9 onward emits all rows. Budget 1 pays the call,
   but no multiplier or return. Derive return targets from reachable call
   contexts, retaining justified fault/HALT edges. Arithmetic or name-lookup
   optimization alone does not fix this reachability defect.

All rows plus successor XORs total 237,923 gates, with histogram
`{G1:30408,G2:32763,G3:176,G4:174576}`. Add 68 dispatch comparators,
36 pc-exchange gates and two HALT-constant XORs to obtain 238,029. These
dispatch comparators have six controls. This reconciles row and layer counts.
The bead's ~one million/step and ~two billion at 2048 are roughly twice this
baseline's full logical-node count. The size defect remains severe; do not
reconcile units by calling every controlled X a Toffoli.

`bennett-circuits-55q` remains relevant: `qasm::order`/export repeat linear
`Program::circuit` and position searches, `QubitRef` owns bundle Strings,
and per-node serialization clones the enclosing circuit in `one`. These
affect construction work and host representation, not the gate equation.
Caching names alone leaves an oversized circuit.

For one circuit, `one(c, vec![node.clone()])` clones the whole N-node Circuit
before replacing its body, once per node. That alone copies N² body nodes
during serialization, independently of dispatch specialization. This is a
deterministic construction-work defect in addition to output size; streaming
must eliminate it rather than merely shrinking its constant factor.

## 3. Engineered step-unrolled exporter

### Preparation and specialization

Keep CLI validation and all published entry capabilities. Index symbols,
blocks, locations and wires once with dense IDs. Bind the internal plan to
the original object/closure identity, arguments, budget and inherited controls.
The plan is an optimization artifact, not a rewritten reference object.
When a specialization proof is unavailable, general calls, recursion, memory
and nested oracles use a budget-bounded machine plan with correct general
kernels. Report the chosen plan and counts explicitly; Collatz is no whitelist.

Inline effects for acyclic static call contexts and bind return destinations
directly. Retain virtual call-record-size, monotonic sp and depth checks.
Prove those values constant before deleting their quantum storage. A depth-one
call need not fit: here successful entry needs stack≥28 and depth≥1.
`(B,0,27,1)` faults with code 3 on its first executed row, while B=0 gives
code 1. At `(B,0,32,32)`, record/counter effects are statically safe and
private; argument capture and return delivery become direct bindings/updates.

Compute S_t over `(canonical pc, call context)` at each original row position,
with sound abstract constants/known bits and fault edges. Here `ret` reaches
wrapper row 1, never every row. Join conservatively; widening must not drop
behavior. For a fixpoint analysis retain an order-independence receipt:
original, function-reversed **and block-reversed** evaluations agree.
Indirect jumps/recursive contexts can widen. Use periodic/shared summaries
for large B instead of storing every hypothetical path; expanded output gates
still count individually.

Fold operands only after dominance/path/write analysis. Loop constant rows
may have no quantum effect while still consuming their original budget
positions. Preserve modular widths, casts, arithmetic faults and first-fault
order. Select fixed-shift aliases, shifted-add multiplication by 3,
increment networks, bit-mask aliases and demand-driven equality (do not
compute an unused `ult` alongside `eq`). General operations remain available.
Fixed and variable shifts, including arithmetic shift≥N, retain ISA zero
results rather than host masked-shift semantics.

### History, cleanup and streaming

Initial policy: retain overwritten live values and irreversible branch/fault
decisions in private clean history. Uncompute nonretained scratch locally
only while its sources/controls are available. Emit
`forward; commit; inverse(forward)`. Workspace sharing needs liveness and
cleanliness proofs. No measurement/reset, discarded history, borrowed-as-clean
assumption, or arithmetic inverse of a non-injective update is allowed.
Streaming changes host storage, not quantum history needs: initial quantum
workspace may be O(B W). Future pebbling must count its extra gates.

Use an indexed immutable plan and deterministic replayable emission recipes.
Each carries wire IDs, controls, constants, source charge and history layout.
Emit forward blocks and commit, then replay blocks/operations in reverse
with adjoints, reproducing wire IDs without retaining every gate object.
A private seekable spool is allowed for reverse traversal or declaration
layout. Validate the plan before stdout; late I/O errors fail explicitly.
File output uses temporary-file completion. No success on truncated output.

Add a `Write`-based QASM API; keep the String API as a small-caller adapter.
Do not flatten a whole circuit for serialization or encode every gate twice
in a giant metadata String. Version compact plan/layout/source annotations
and migrate existing canonical `qasm::parse` consumers explicitly. Keep
independent executable parsing: legacy IR roundtrip equality remains for
legacy format; executable equivalence is the invariant of the new format.
Label source certificates as source evidence, with fresh binding/validation
for optimized target claims.

### Checked count model

Use `G_step(B) = 2 Σ_t [dispatch(S_t)+Σ(kernel+update+successor)] + commit`,
with bounded return edges and specialized operands. Constant rows still
advance t. Predicates, updates and history reversal all count.

Two scratch models reuse baseline row routines with literal reads, elide
constant assignments, substitute eight direct argument/return gates, and
change return reachability to `{1,HALT}`. The first keeps the inefficient
literal-shift kernel; the second uses fixed-shift aliasing. These are
**candidate cost models**, not proved circuits: lifetime/history bindings,
predicate pruning and general-case selection still need implementation and
differential verification.

| B=2048, `(heap,stack,depth)=(0,32,32)` | Full logical gates | Evidence |
|---|---:|---|
| Current lowering | 971,566,939 | Exact code projection |
| Literal reads + bounded returns + static call effects | 992,159 | Candidate model |
| Same, fixed-shift alias | 819,335 | Candidate model |
| Bead's step estimate | ~4,000,000 | Prior estimate |
| First step acceptance | ≤4,097,000 | Proposed requirement |

The alias model has 409,660 forward gates, their inverse and 15 commit gates:
about 400 full gates per budget unit at 2048. Thus ~2,000/step is a useful
conservative target here, not a reproduced exact per-step constant. Further
simplification can reduce this; proven cleanup/replay may add gates. The
allowance is neither a decomposition budget nor a bound on other programs.

## 4. Loop-level alternative

Recognize natural loops/live state, compile a reversible iteration with a
coherent active/halted predicate and retained decisions, and unroll the maximum
potentially successful iterations. This removes dispatch and transient-register
transport. It does not change the budget unit to iterations or use a quantum
QASM `while`.

Here the nontrivial path initializes in eight rows. A continuing iteration
is rows 12..29: **18 rows**. An exiting iteration takes rows 12..27 and
30,31,32,1: **20 rows**, including ret/retval. Success in iteration j needs
`B≥10+18j`; the direct x=1 path succeeds at B=9. At B=2048 at most
`floor((2048-10)/18)=113` iterations can succeed. These exact structural
counts do not assume a maximum mathematical Collatz length. Zero and
nonterminating/overflowing modular trajectories remain in D.

The candidate state is `(v3:i64,v2:i8,active,outcome)` plus retained old
values. The payload counter wraps at eight bits and recurrence at 64 bits.
Freeze first success and include entry/exit charges. This fixture's incomplete
last iteration cannot expose an intermediate arithmetic fault, so when no
success is possible before the cutoff its outcome is code 1. General loops
must preserve faults inside partial iterations: use an exact residual-prefix
machine or per-row budget guards.

| Per-forward-iteration component | Gates | Basis |
|---|---:|---|
| `3*x mod 2^64` | 437 | Existing literal-3 Build kernel |
| Add one at width 64 | 189 | Existing increment kernel |
| Select even/odd result | 320 | At most 5×64 |
| Increment width-8 counter | 21 | Same 3N−3 increment formula |
| Equal-one test | 63 | N−1 ANDs over polarity bits |
| Retain/update 64+8 state bits | 288 | Copy plus 3-X exchange, 4×72 |
| Active/exit/budget predicates | 128 | Engineering allowance |
| Total | 1,446 | Estimate |

Parity and fixed right shift use aliasing. Kernel intermediates stay retained
until global reversal; no free local cleanup is assumed. Allow 50,000 gates
for entry, exits, commit and boundary handling:
`G_loop(2048) ≈ 2×113×1446+50000 = 376796`, consistent with the bead's
~400,000. The 128 and 50,000 allowances need actual planner receipts:
this is **not** a proved upper bound. Inherited active controls increase
control arity/decomposed cost, which must also be reported. Gate-definition
sharing reduces text, not the expanded 113 invocations.

Land the step plan first: it preserves row boundaries directly and supports
general loops with internal faults, memory, indirect control or recursion.
Loop specialization remains required follow-on engineering with separate
equivalence/count evidence, not a restriction on exportable oracles.

## 5. Semantic differential and fixtures

Freeze existing `oracle::specialise` as the legacy/reference construction.
Do not teach it the same optimizer or derive expected values from the new
plan. Tests use the **same checked Object, arguments and budget** with both
builders, independently export/parse executable QASM and simulate it. Invoke
the gate-pinned ISA reference to adjudicate payload, code and source steps.
Spec/legacy disagreements are reported and tracked, not silently preserved.

For permutation oracles, use a reversible bit-vector evaluator over actual
parsed X/control operations that inspects **all** allocated bits, including
workspace. Reject non-permutation gates in this path. Exhaustive basis equality
plus absence of phases/measurement proves linear equality, including entangled
inputs. Existing exact-amplitude tests cover general actions/controls/phases.
Workspace widths may differ: each implementation must restore its own private
state and match the entire interface. Metadata/text equality is insufficient.

First fixture batch:

- Collatz legacy differential: B=`0,1,2,4,8,9`, `(heap,stack,depth)=(0,32,32)`.
  These span call, first return and first successful retval. Exhaust x=0..255.
  For each x use `(target,fault)=(0,0),(255,15),(85,10),(170,5)` and every
  target/fault singleton bit, with structural read-only/XOR-output checks.
  This is exhaustive over x, not the whole 20-wire ABI; do not claim otherwise.
- Capacity boundaries: `(stack,depth)=(27,1),(28,0),(28,1),(32,32)` at
  B=0,1,9. The first two produce code 3 at B≥1, but B=0 gives code 1.
  Launch-invalid limits and execution exhaustion are different conditions.
- Collatz loop/success boundaries against pinned ISA, avoiding large legacy
  materialization: B=`10,27,28,29,45,46,47,2048`, all x and the probes above.
  x=1 first succeeds at 9; x=2 at 28; x=4 at 46, with payloads 0,1,2.
  x=0 never succeeds. Obtain the other outcomes independently from reference.
- Small fixtures: at most four input bits, two target bits, four fault bits,
  tiny input-memory/borrow pools as needed. Exhaust the **entire** interface
  domain, including all memory cells and nonzero target/fault. Budgets
  `0..=12`, plus first-success/fault step minus one/equal/plus one.
  Cache circuits by object/arguments/budget, never by input tuple.

Small cases isolate constants/mul 0,1,3 and modular overflow; fixed/variable
shifts at 0,N−1,N,N+1 including signed all-ones; select/parmov aliases;
diamonds/unreachable rows; two call sites, depth-one and recursive calls;
monotonic stack exhaustion after return; labels and bad qjumpr; divide-zero,
signed overflow and competing first faults; partial-loop faults; tiny memory,
alloc/free including pinned entries; nested oracle call/uncall; inherited
positive/negative controls and nontrivial borrowed state. Every optimization
needs a test that fails when deliberately corrupted. Expectations are
hand-derived boundaries or independent reference outputs, not new-plan output.

Direct legacy simulation can bypass the old writer's name/clone overhead;
small fixtures still test actual QASM. Large Collatz tests use the engineered
writer and an independent streaming executable reader/evaluator rather than
materializing another enormous `Flat` for validation.

Design verification already run: the scratch `verify-design.py` checks the
counter's exact source extraction, fixture bytes/hash, all reported cost
receipts/equations and the five-question limit. It also ran **11 pinned ISA
reference boundary cases**, confirming x=1 at B=8/9, x=2 at 27/28, x=4 at
45/46, x=0 at 2048, and four call-capacity/B=0 cases. All passed. These are
design evidence; the broader differential/gate matrix above is future work.

## 6. Acceptance and gate integration

The bead's “no gate exercises export-qasm” claim is stale at this baseline.
`gate.toml` runs integration tests; `tests/integration.rs` includes
`tests/exporters.rs`, whose `cli_exports_actions_tokens_and_bounded_oracles`
exports `r9-sol-oracle.bisa @o` at `1,0,0,0`. `tests/qasm.rs` covers actions,
hierarchy and stale-identity rejection; differential tests parse action QASM.
The gate explicitly runs ignored oracle tests, exercising literal
specialization, but this does not establish compiled Collatz QASM size.
`scripts/run-conformance` invokes suite F without a separate export/count
check. The real gap is realistic compiled-oracle export and deterministic
size/representation regression coverage.

Put the following **non-ignored** tests in the existing integration target
so normal `cargo test` executes them. Add a named gate check only if the
harness cannot live there; no second full gate or silently ignored regression.

| Acceptance | Required receipt |
|---|---|
| Semantics | Zero divergences for defined domains/budgets; private cleanup, no phase, involution and controlled-call checks |
| Red on baseline | B=1: 184,677 >3,000; B=32: 11,834,011 >65,000. Count nodes/gates directly without elapsed assertions |
| Step size | `G(B)≤2000 B+1000` at B=0,1,2,4,8,9,16,32,64,2048, fitting budgets; include forward/commit/inverse and expanded calls |
| Reachability/kernels | No stack scans for the proven single-call context; no generic multiply/variable-shift kernels for literal 3/1; emitted row-contexts match sound planned sets |
| Representation | Peak resident host gate objects ≤50,000 at all size points; doubling B does not duplicate resident gate arrays; separately count plan descriptors and history wires |
| Construction work | One writer visit per emitted operation, one replay visit per inverse operation; indexed names, no per-gate program scan or enclosing-circuit clone |
| Stream integrity | Parsed executable count equals plan count; complete independent parse; injected I/O/allocation errors fail explicitly, no truncated-output success |
| Loop follow-up | Exact boundary/fault equivalence, then ≤400,000 expanded gates at B=2048; openly re-estimate if implementation disproves the target |
| Generality | Existing action/oracle/draft-opt-in/control/memory cases stay green; correct general kernels remain available without specialization proofs |

The host-object limit is not a quantum-workspace limit or spool-byte limit.
Report analysis visits, resident descriptors, peak gate objects, total/peak
history wires, maximum controls and G_c. Specify decomposition before claiming
hardware-resource improvements. Ratchet these work counts with correctness;
validate new syntax with the external OpenQASM parser and actual executable
semantics, not only annotations. Allocator caps and elapsed guards never
become semantic oracle budgets. Implementation runs need the repo hang-guard
class table; a trip is UNKNOWN, not a failed semantic/size assertion.

## 7. Staged implementation and ownership

These are future assignment boundaries, not authorization for this child.
Each starts with red evidence, focused verification, and the required same-head
review/gate disposition. Only stage 0's design is delivered now.

| Stage | Future owned paths | Exit evidence |
|---|---|---|
| 0: council | This doc; council review docs assigned separately | Disposition on the five questions before implementation |
| 1: freeze/count/test | `src/oracle.rs` (access/API only; legacy unchanged), `tests/oracles.rs`, `tests/exporters.rs`, new `tests/export_oracle.rs`, new `tests/fixtures/export_oracle/**`, `tests/integration.rs` | Checked/hash-bound Collatz fixture, independent evaluator, exact baseline counts, red size cases, legacy semantics green |
| 2: analyzed plan | New `src/oracle_plan.rs`, new `src/oracle_plan/**`, `src/lib.rs`, `tests/export_oracle.rs`, fixture directory above | Context reachability, virtual budget events, constant proofs and order-independence receipts |
| 3: reversible recipes | New `src/oracle_emit.rs`, new `src/oracle_emit/**`, plan paths above, `tests/export_oracle.rs`, `src/oracle.rs` (dispatch API only) | History/reversal proofs, specialized kernels, semantic differential and actual step gate envelope |
| 4: stream/CLI | `src/qasm.rs`, `src/main.rs`, `src/ir_codec.rs` (if annotations change), `src/export.rs`, `tests/qasm.rs`, `tests/exporters.rs`, `tests/export_oracle.rs`, `scripts/validate_exporters.py`, `docs/EXPORTERS.md` | Indexed writer, bounded gate objects, independent parse, injected I/O failures, action compatibility |
| 5: gate coverage | `gate.toml` only if needed, `tests/integration.rs`, `tests/export_oracle.rs`, `tests/differential.rs`, `scripts/run-conformance`, `docs/EXPORTERS.md` | Named non-ignored semantics/size/stream tests in normal gate; spec-owned suite F unchanged, local export differential explicit |
| 6: loop plan | Plan/emit modules above, `tests/export_oracle.rs`, loop fixtures | Source charges/residual prefixes/first faults proved; actual ≤400k target assessed |

Shared paths overlap across **sequential** stages. These are not disjoint
parallel assignments: one writer owns shared tests/plans at a time. Any split
requires explicit disjoint ownership in both briefs. Do not read/copy sibling
implementation code across a contract boundary or publish operational logs.

## 8. Council questions (sol, astra, Muse)

1. **What remains invariant?** Recommend extensional equality on all D under
   the same identity/arguments/budget, exact phase and cleanup; allow private
   history and gate-byte changes. Keep source/target proof bindings distinct.
2. **May folding/inlining remove reference charges or call faults?** Recommend
   no: preserve virtual charges, monotonic stack accounting, observable labels
   and first-fault priority. Only statically discharged physical checks vanish.
3. **Which history policy first?** Recommend retained overwrite/decision
   history with global inverse replay; local uncompute needs restored-source
   proofs. Initially accept linear quantum history, but stream host gates and
   report both resources.
4. **Which size evidence establishes fitness?** Recommend expanded logical
   gates plus G_c/arity, quantum wires, resident IR objects and construction
   visits; step envelope `2000 B+1000` on Collatz. Treat ~4M/~400k as same-unit
   targets, never measured Toffoli/T counts; semantics gates every reduction.
5. **Should loop lowering replace the step plan immediately?** Recommend no:
   land verified step specialization and retain its general fallback, then
   prove loop budget/fault boundaries with residual prefixes where necessary.
   Preserve independent small-budget legacy lowering throughout.

## Appendix A. Reproducible input

The supplied object follows. Preserve all 54 lines and trailing newline for
the SHA-256 above. Eight-bit interface and internal 64-bit arithmetic coexist.

```bennett
bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=64

section symbols
{"0":{"interface_hash":"2ab39c58aeb271ede5f7c6a4b29021f7c40c131d60ee4fca42f3410538d0394e","kind":"oracle","name":"ref","signature":{"abi":"0.8","args":[],"fault":4,"in":[8],"kind":"oracle","mem":[],"tgt":8,"tier":"q"}},"1":{"interface_hash":"763cca54458f1f9818a8e1aecae275e70dada9f4367d70eabfc0addd74d63c17","kind":"lproc","name":"L1","signature":{"abi":"0.8","kind":"lproc","params":[8],"result":8,"tier":"q"}}}

section functions
oracle @ref (x0:wire[8] in) (tgt:wire[8]) (fault:wire[4]) () ()
regs { r:reg[8] }
^entry():
r = lift.call @L1 (x0)
lift.retval r
lproc @L1 (a0:reg[8]) (r:reg[8]) () () ()
regs { c0:reg[8], v0:reg[1], c1:reg[8], v12:reg[8], v1:reg[64], c2:reg[8], v2:reg[8], v3:reg[64], c3:reg[64], v4:reg[64], c4:reg[64], v5:reg[1], c5:reg[64], v6:reg[64], c6:reg[64], v7:reg[64], c7:reg[64], v8:reg[64], v9:reg[64], c8:reg[8], v10:reg[8], c9:reg[64], v11:reg[1] }
^b0():
c0 = lift.const.i8 1
v0 = lift.icmp.eq a0 c0
qbranch v0 ^b0.e3 ^b1
^b0.e3():
c1 = lift.const.i8 0
(v12) = lift.parmov (c1)
qjump ^b3
^b1():
v1 = lift.zext.i64 a0
c2 = lift.const.i8 0
(v2, v3) = lift.parmov (c2, v1)
qjump ^b2
^b2():
c3 = lift.const.i64 1
v4 = lift.and v3 c3
c4 = lift.const.i64 0
v5 = lift.icmp.eq v4 c4
c5 = lift.const.i64 1
v6 = lift.lshr v3 c5
c6 = lift.const.i64 3
v7 = lift.mul v3 c6
c7 = lift.const.i64 1
v8 = lift.add v7 c7
v9 = lift.select v5 v6 v8
c8 = lift.const.i8 1
v10 = lift.add v2 c8
c9 = lift.const.i64 1
v11 = lift.icmp.eq v9 c9
qbranch v11 ^b2.e3 ^b2.e2
^b2.e2():
(v2, v3) = lift.parmov (v10, v9)
qjump ^b2
^b2.e3():
(v12) = lift.parmov (v10)
qjump ^b3
^b3():
lift.ret v12

section layout
{"A_tab":0,"H":12,"S_cells":5,"globals":{},"heap_base":4096,"heap_limit_max":0,"interface_regions":{"ref":{}},"null_size":4096,"ptr_width":64}
```
