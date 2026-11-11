# Circuit exporters

OpenQASM streams proved bounded-oracle recipes directly. Its lossless
`ir::Program` API remains available for actions and general constructions.
QIR and Quil retain their `ir::Program` adapters, including the same engineered
oracle lowering; those adapters still materialize their instruction arrays.
The CLI checks stored identities and source obligations before every format.
Classical arguments and budgets use the same syntax in every format:

```sh
bennett-circuits export-qasm object.bisa action
bennett-circuits export-qir object.bisa action --arg token:1,0,0,0
bennett-circuits export-quil object.bisa oracle --budget 4,0,0,0
```

An oracle requires an explicit budget. An action receives tokens through
`--arg`, in signature order. Embedded schedules retain the common §11 library
check; choosing another export format cannot bypass it. Draft certificate
input still requires the existing explicit draft opt-in.

QIR and Quil flatten the shared hierarchy into executable instructions. Their
versioned `bc:` comments preserve the complete source IR, including allocation
kinds, call hierarchy, own controls and attached source certificates. A separate
`bc-layout:` comment describes physical indices and the interface/workspace
partition. This is transport of source proof material, not a certificate for a
new target or schedule. Workspace and borrowed pools use ordinary qubits; a
standalone zero-initialised run has the same entry convention as `run-action`.
A caller can initialise the declared physical wires before invoking the body.
A zero-wire identity requires no sentinel in either format.

`qir::parse` and `quil::parse` recover the exact hierarchical IR and check the
canonical executable against it. Their separate `parse_flat` implementations
read actual gate calls/instructions and the layout, ignoring the IR comment.
These are validators for the emitted subsets, not general import APIs. They
reject unknown instructions, invalid indices, operand aliasing, malformed
angles and incomplete output. Removing an IR comment does not prevent
executable simulation; changing executable gates is visible to that simulation.

## OpenQASM streams and file output

```sh
bennett-circuits export-qasm collatz.bennett ref --budget 2048,0,32,32 --output collatz.qasm
```

Omit `--output` to write stdout. Every format accepts `--output PATH`: it writes
a sibling temporary, flushes and synchronizes it, then atomically replaces the
destination. An error preserves the previous destination. Stdout write/flush
errors produce a failing exit; consumers must check completion before using
an oracle artifact. Source validation finishes before stdout receives a header.

`qasm::write` emits the legacy hierarchical format through `Write`, with indexed
names and lazy primitive expansion. It neither clones the enclosing circuit per
node nor constructs a flattened gate array. Legacy `bc-ir-3` metadata is written
one node at a time. `qasm::export` is the retaining String adapter;
`qasm::parse` preserves exact legacy IR roundtrips.

`qasm::write_oracle` consumes immutable recipes and their bound checked Object.
It emits `qubit[N] q;` and direct `ctrl`/`negctrl` standard-gate statements, using
one recipe layer at a time. No whole oracle Program, gate array, gate-definition
argument list or output String is constructed. `--output` and stdout both use
this path when specialization proofs apply. General oracles and actions retain
the lossless hierarchy path; choosing streaming never removes a capability.

The compact comment version is `bc-oracle-stream-1`. `bc-stream` records the
original object/closure identities, arguments, original budgets, a `bc-ir-3`
layout with an empty body, and logical gate/history counts. Integers in compact
counts/bindings are decimal strings. This is source identity evidence; the
artifact carries no claim that an old source certificate proves a new target
realization. Target and schedule certificates require fresh binding/validation.

A mandatory `bc-end` record carries the executable count and `sha256-chain-v1`
completion digest. Starting at SHA-256 of UTF-8
`bennett-qasm-sha256-chain-v1`, each exact line before the footer updates the
chain as SHA-256(previous digest || 8-byte big-endian line length || line bytes).
Newline bytes participate. This detects incomplete/edited transport, not a
producer attestation or a proof of semantic equivalence.

`qasm::visit_stream_checked` validates the compact schema, layout, actual gate
count/control histogram and completion digest while visiting actual statements.
A visitor must discard its result on error, including an error detected at EOF.
`qasm::visit_stream` independently reads the direct executable subset, ignoring
IR/source/completion comments. `parse_flat` also accepts that subset, including
metadata-stripped executable text, while retaining its legacy hierarchy parser.
`parse` reconstructs a compact artifact's Program from layout plus executable
statements and validates completion; executable equivalence is the compact
format's invariant. Materializing parse/String adapters remain small-caller
interfaces; the visitor APIs retain one parsed gate at a time.

Writer counters distinguish operation visits, actual bytes, metadata size and
resident recipe/layout descriptors. Peak gate objects include one transient
writer gate beside the recipe layer and exclude a caller-owned legacy Program.
Quantum workspace/history widths are separate metrics. The test host memory
limit is a representation guard, never an ISA oracle budget or a time threshold.

## Proved natural-loop recipes

The default oracle constructor recognizes a single natural loop in the
context-expanded CFG: the header dominates the cycle, the body and continuing
arm are straight total paths, and the other arm reaches one terminal outcome.
It computes live loop-carried words from reads before writes. Prefix paths and
static call/return contexts preserve initialization and entry/exit charges.
Names, fixture identities and arithmetic constants are not recognition keys.
A bounded proof search selects the exact row construction when it cannot prove
this shape. Calls in the cycle, conditional faults, memory, recursion and
unproved control flow keep the existing exact construction. In particular,
a fault in an incomplete iteration is never discarded.

Each iteration snapshots only carried words, evaluates internal expressions as
immutable SSA aliases, updates the carried state in parallel, and retains the
active decision. The first terminal outcome freezes coherently. Forward
recipes, output XOR and exact inverse recipes restore every private wire.
The compiler unrolls only terminal opportunities whose original row charges
fit the budget; it emits ordinary gates, with no quantum loop or iteration
budget. `oracle_emit::prepare_step` explicitly retains exact row recipes for
differential checks; `oracle::specialise_legacy` remains independent.

The Collatz proof has 8 entry rows, 18 continuing rows and 20 exiting rows.
At B=2048 there are 113 possible exiting iterations, irrespective of the
mathematical stopping time. Actual forward costs are 24 entry gates and
1,259 per iteration; output XOR is 13 gates. Thus forward is 142,291 and
full forward/commit/inverse is **284,595**, within the 400,000 gate ceiling.
Steady gates per original row change from 239.5 to 1,259/18 = 69.9444 forward,
and from 479 to 139.8889 including reversal. These are expanded logical counts,
not elapsed acceptance or gate-definition compression.

At this budget the full histogram is G1=195,523 and G2=89,072, maximum two
controls. The retained history has 88,267 wires, with 88,353 private wires in
total. Three recipe descriptors and at most 1,259 resident recipe gates
(1,260 including the writer transient) suffice at every tested budget, including
doubling. The CLI representation guard retains B=2048 and also checks B=8192
under the same 128 MiB address-space cap, so reduced gate counts cannot conceal
a newly materialized whole-oracle array.

For an explicit decomposition comparison, use X/CX/CCX and the exact clean
AND-chain implementation of a c-control X: 2c-3 CCX for c>=3, with c-2 clean
ancillas restored. The previous B=2048 histogram (G1=240,397, G2=728,440,
G6=9,090) then gives 810,250 CCX and 240,397 CX, needing four extra clean
ancillas. The loop histogram gives 89,072 CCX and 195,523 CX, with no such
extra ancillas. These are costs derived from the stated decomposition and
measured histograms; no hardware T-count is claimed. Inherited controls add
to arity: the mixed two-control acceptance exercises maximum arity four,
nonzero outputs, inactive calls and untouched external state.

Non-ignored normal tests cover every cutoff 0..65 against exact row recipes
(270,336 input/output tuples per construction), pinned-reference and real
streamed-CLI boundary matrices, exhaustive tiny parallel-alias and wrap cases,
root headers, both exit polarities, partial-iteration faults and inherited
controls. Literal small-budget differential and the existing general corpus
remain required. The normal harness discovery audit includes the loop cases.

## QIR

The output is unrestricted QIR 1.0 using typed `%Qubit*` handles with consecutive
static integer indices, zero result handles and no measurement. It declares
64-bit pointers in its LLVM data layout. QIR profiles and QIS are distinct:
this exporter needs the QIS and runtime below and does not claim Base Profile
compatibility. See the [QIR specification](https://github.com/qir-alliance/qir-spec/blob/main/specification/README.md)
and [instruction-set contract](https://github.com/qir-alliance/qir-spec/blob/main/specification/Instruction_Set.md).

Single-qubit calls use `__quantum__qis__{x,h,s,t,rx,ry,rz}__body`, S/T adjoints,
and controlled/controlled-adjoint variants. Controls use `%Array*` values from
`array_create_1d`, `array_get_element_ptr_1d` and
`array_update_reference_count`; every array is released. Controlled rotations
use the QIR 1 tuple ABI: an array followed by a pointer to
`{ double, %Qubit* }`. These signatures are exercised in the QIR Alliance's
[controlled-signature tests](https://github.com/qir-alliance/qir-runner/blob/main/runner/tests/controlled_signatures.rs).

Negative controls use X conjugation. CZ appends its first target to a controlled
Z's controls; SWAP uses three controlled X calls. These identities introduce no
rotation arithmetic or global-phase approximation. Other gates retain their
original QIS operations. Angles are exact hexadecimal binary64 bit patterns,
including signed zero and subnormals. Array memory is classical; no extra
quantum ancillas are allocated by the exporter.

## Quil

The output uses the standard X/H/S/T/CZ/SWAP/RX/RY/RZ gates, `DAGGER` for S/T
adjoints, repeated `CONTROLLED` modifiers and X conjugation for negative
controls, followed by `HALT`. Rotation arguments use round-tripping decimal
binary64 literals. See the [Quil specification](https://quil-lang.github.io/).
No custom gate matrices, approximate decompositions or extra qubits are needed.

## Validation

`tests/exporters.rs` covers every gate, control polarity/count, adjoints, tiny
angles, zero-wire output, CLI actions/tokens/oracles, stale metadata and direct
executable mutation. LLVM 18 `llvm-as-18` and `opt-18 -passes=verify` validate the
all-gates QIR when installed; absence is reported explicitly. The job separately
assembled and verified the same module with LLVM 23 (`llvm-as-23`, `opt-23`).
The job also checked for Quil/pyQuil, quilc, QVM, pyqir and qirrunner: none
was installed.
LLVM validation establishes LLVM well-formedness, not quantum execution.

`tests/differential.rs` discovers all reference fixtures and adds supplemental
calls, workspace, memory and gate cases. It simulates the independently parsed
QIR, Quil and QASM on every interface basis tuple and compares exact states,
including phase and workspace, against the unexported circuit and placement
passes. Reference CLI display values are compared at absolute tolerance 2e-12;
that display comparison is evidence, not a numerical proof. The current corpus
contains 24 actions and 940 exhaustive basis tuples, including explicit
one-step token bindings for the re-imported oracle fixtures.


The test profile uses optimization level 1, with debug assertions and overflow
checks explicitly enabled. This keeps the existing multi-million-gate oracle
checks practical without reducing their domains or checks. The initial
unoptimised full run exceeded its 900-second bound; final results are recorded
in the job Resume block and Beads closure reasons.


## Acceptance commands and gate ownership

The normal `gate.toml` cargo-test check discovers the existing `integration`
target without a filter. Its non-ignored `export_oracle::` module owns the
compiled-oracle acceptance: checked fixture/legacy baseline, constants and
context reachability, virtual budget/fault boundaries, complete private cleanup
and involution, logical size limits, resident representation and streamed CLI
agreement with the gate-pinned ISA. `qasm::` and `exporters::` add transport-error,
allocation-error, metadata-tamper and independent grammar checks. A runtime
harness-discovery test verifies that these named acceptance cases are present
and not ignored. No second full gate or exporter-only gate check is added.

```sh
cargo test --offline --test integration export_oracle:: -- --test-threads=1
scripts/run-conformance
```

`run-conformance` compiles and invokes the pinned published harness unchanged,
with its original cases, `--suite F`, binary and forwarded arguments. It then
runs the named local exporter module separately, including when F fails. The
published harness stdout and verdict are retained. Local diagnostics and its
PASS/FAIL/UNKNOWN label go to stderr. A failed published F keeps its original
exit priority; otherwise a local failure makes the combined command fail.
Thus neither a passing source checker nor a passing exporter can hide the other
failure. The local tests are consumer evidence, not new spec-owned F vectors.
A published unavailable verdict also retains its original nonzero status; local
success never relabels it as a source-conformance pass. At pin `ecaaac060c44`,
F reports 33 PASS, 0 FAIL and four published staged UNAVAILABLE probes
(`F-frame-live-ranges`, `F-frame-records-09`, `F-link-conflict`,
`F-ratified-confirmations-09`). Probe/version adoption is separate work; the
link-conflict staging text predates the consumer's existing link command.

Local command hang guards are distinct from acceptance budgets. The exporter
module has a 1200-second guard (`BENNETT_EXPORTER_GUARD_S` for an admitted host
profile); a timeout or interruption is UNKNOWN and exits nonzero. Gate-run
commands retain the repository's configured 7200-second guard and admission
policy. These protect a host from a hang; they never change original source-row
charges, gate ceilings or the oracle input domain. Keep resource admission and
host-quiet holds outside the conformance command. Loop equivalence and the
400,000-gate B=2048 ceiling are separate normal acceptance assertions; passing
the step exporter alone does not claim the loop target.


| Verification class | Existing command owner | Work acceptance | Hang protection |
|---|---|---|---|
| Normal exporter acceptance | `integration` target in the normal cargo-test check | Named non-ignored semantics, size, representation, transport and external grammar cases above | Gate command guard: 7200 s |
| Serial local export differential | `run-conformance` after unchanged published F | `export_oracle::` module, exact reference agreement and original budget boundaries | 1200 s; override only for an admitted host profile; timeout/signal is UNKNOWN |
| General oracle differential | Existing post-merge `oracles` check | Published oracle corpus, cleanup, reversal and CLI agreement | Gate command guard: 7200 s; approximately 7 GiB admission class |

The serial profile uses three offline build workers and one test thread; the
normal gate uses three build workers and four test threads. Test optimization,
assertions and overflow checks are unchanged. These rows assign protection to
the commands that already own the work, rather than adding an independent gate
or replacing any published conformance requirement.
