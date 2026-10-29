# bennett-circuits — plan

The bounded circuit specialiser for the Bennett virtual ISA: given an ISA
object (§9/§16 of `../bennett-isa-spec/spec/Bennett-ISA-Spec.md`, ISA 0.8), a
region, a classical budget (§5.4) and classical parameters, produce a circuit
that is the region's semantics under that fixed budget — "the circuit
specialiser is the VM's semantics under a fixed budget, no more and no less"
(NorthStar §4.2) — plus placement, proof-carrying output (§12), and exporters
(OpenQASM 3, QIR, Quil). This repository is the second executor of the north
star: rungs 1–2 of the §8 demo ladder require both executors; rungs 4, 7 and 8
require compiled oracle circuits at small instance sizes (their resource
estimates at scale are `bennett-estimate`'s, from our certificates); rungs 3,
5 and 6 are primarily the VM's reversible tier and need circuits only where a
coherent region is specialised. OpenQASM/QIR/Quil export is the interchange
surface for external simulators and hardware bring-up, not a rung itself.

Contracts only, never shared code (CLAUDE rule 5): inputs are the published
ISA spec's object/target/schedule/certificate documents; the scheduler is
consumed through the §11 C ABI as data (a schedule document in, verbatim
execution), never linked. The reference interpreter binary is a differential
oracle through its CLI (`check`, `run-action`, later `run-oracle`,
`step-check`) in tests only.

## Architecture

One Rust crate, std only. Pipeline:

```
.bisa text ──▶ object (loader, §16 grammar; identity, §9.2; check)
           ──▶ lower  (Q actions now; oracle specialisation in M2)
           ──▶ ir     (hierarchical circuit IR)
           ──▶ flatten/placement (wire allocation; later classes/schemes)
           ──▶ export (OpenQASM 3 now; QIR, Quil later)
           ──▶ cert   (§12 certificate generation; M4)
```

- `src/sha256.rs`, `src/json.rs` — SHA-256 and a JSON/JCS (RFC 8785 subset:
  objects, arrays, strings, integers, booleans, null; non-integer numbers are
  a loud load error in hashed contexts) implementation. Everything the object
  format hashes goes through these; nothing else in the crate parses JSON.
- `src/object.rs` — §16 parser into sections, symbol table, function ASTs
  (full AST for Q actions; structural parse — header, regs, blocks, raw
  instruction lines — for oracles, lprocs, CC and R, which is what identity
  needs), layout/data/imports. §9.2 identity: canonical function bytes,
  semantic envelopes, closure sets per symbol kind, SCC handling (a trivial
  SCC hashes its envelope directly; a cyclic SCC wraps via `scc_hash`),
  interface and closure hashes, `object_hash`. `check` = parse + schema and
  signature consistency + stored-hash verification (see milestone 1 scope).
- `src/ir.rs` — the hierarchical circuit IR (below).
- `src/lower.rs` — ISA Q-action bodies → IR. Predication (§3.5) becomes gate
  controls; `qif`, `qloop`, `qloop … while` expand by their §4.6 constructions
  (bounded unrolling is the specialiser's job); calls stay calls.
- `src/flatten.rs` — IR → flat gate list over allocated qubit indices:
  recursive call expansion with inherited controls, bundle-op and
  memory-op expansions (§4.5/§4.6 address dispatch). Milestone-1 placement is
  naive static allocation: interface wires first (in `run-action` order:
  mut bundles, in bundles, mem cells; wire 0 of a bundle is its bit 0), then
  one fresh allocation per workspace bundle per call instance, no reuse.
- `src/qasm.rs` — OpenQASM 3 exporter and a parser for exactly the subset it
  emits (for the round-trip test). Circuits export as `gate` definitions;
  allocation structure that QASM cannot express (workspace kinds, local-region
  geometry) is carried in `// bc:` comments, which other tools ignore.
  Borrowed wires export as ordinary qubits: a top-level export instantiates
  the borrow pool in |0⟩, matching `run-action`; the general dirty-borrow
  contract (§5.6) is a property of the IR, recorded in certificates from M4.
- `src/main.rs` — CLI: `check <object>`, `hash <object>`,
  `export-qasm <object> <action>`.
- `tests/` — maintained suites: identity/check agreement, lowering unit
  tests, QASM round-trip, and the differential harness with its own sparse
  state-vector simulator (`tests/sim.rs`, used by `tests/differential.rs`).

### The IR

A `Program` is a set of named `Circuit`s plus a top circuit. A circuit has
interface bundles (`mut`/`in` wire bundles, `mem` regions with `[A, W]`
geometry and role), workspace bundles (clean `qalloc`/`qcopy`/`qload`
products, `qborrow`ed bundles, local-region cells, loop flags — each declared
once; the specialiser alpha-renames on unrolling), local-region geometry, and
a body of nodes:

- `Gate` — a §4.4 base gate (`x h s sdg t tdg cz swap rx ry rz`) with a list
  of controls (qubit + polarity; `cx`/`ccx`/`mcx` are `x` with 1/2/k positive
  controls) and targets. All predication is here.
- Uncomputation pairs, kept explicit so later scheduling can choose
  realisations (§6.4): `Copy`/`Uncopy`, `Load`/`Unload`; self-adjoint bundle
  ops `Xor`, `Exch`, `Store`, `ExchM` (§4.5/§4.6), each carrying its controls.
- `Call` — callee name, adjoint flag, extra controls (from `ctl`), argument
  bundles in callee interface order. Hierarchy is never flattened in the IR;
  `flatten` and the exporter decide how to realise it.
- Allocation/release are declarations, not nodes: hoisting clean workspace to
  circuit entry is unitary-equivalent, and `qfree`/`qreturn` cleanliness
  becomes an end-of-circuit obligation that the simulator checks and the
  certificate will claim. Local-region image initialisation and its reversal
  are explicit `x` gates (§4.6); under `ctl`, flatten controls every gate
  including image gates (§5.3 "every gate of the body and its cleanup runs
  under w"), while allocation itself stays unconditional.

`adjoint(circuit)` implements the §5.3 table on nodes (reverse order;
`rx/ry/rz(θ)→(−θ)`, `s↔sdg`, `t↔tdg`, pair swaps, call-adjoint flip).

### The bounded specialiser (oracles)

Milestone 2. The reference realisation is §5.2's construction literally: step
state, locations, local regions, allocator pre-entries and B frames; B
forward §3.6 steps (dispatch, row execution with assignment/capture slots,
successor, commit-pc, dispatch cleanup); the commit XORs; B inverse steps.
Gate-level decompositions (adders, comparators, `inrange` scans, multiplexers)
come from the expansion library bound by the schedule (§11; default `ref`,
§17 item 7) — the IR therefore needs operation ordinals stable under the
§5.7 transformations (inlining, structure recovery, dispatch re-indexing),
which are later optimisations gated by T6 differential tests against the
literal construction. Classical parameters and budgets are specialisation
inputs: the same body under two budgets is two circuits (and, for actions,
two identities).

### Placement and scheduling

Milestone 3. Liveness-based wire reuse, qubit-class assignment from a target
description (§8: classes, gate schemes with `epsilon` matching, QRAM
capability), borrow-pool management (callees borrow only from the caller's
designated idle set, §9.4), frame descriptors as functions of inherited
control count `c`, and verbatim consumption of §11 schedule documents
(cleanup realisations `reverse`/`recompute` per §6.4 — measurement-based
cleanup is not admitted in 0.8 — gate realisations and epsilon shares,
concrete alloc classes, start times). Resource vectors are charged through
the target's own expressions, never hard-coded.

### Proof-carrying output

Milestone 4. Emit §12 certificates bound to closure hashes: per-region frame
descriptors, resource vectors, `gates_used`, cleanup entries keyed by
`(variant, inherited_controls)`, error composition (local/inclusive,
operator-norm vs diamond conversion), and claims — `exhaustive` (proof, over
the full domain D of §5.2/§12.2) at small widths, `sampled` (evidence, with
seeds and counts) above them, `structural` for borrow frames and adjoint
tables, `numerical` for ε > 0 realisations. Proof and evidence are never
blurred (NorthStar §5.1). Until then this crate emits no certificate and its
differential harness is test evidence, labelled as such.

### Exporters

OpenQASM 3 first (milestone 1): hierarchical `gate` definitions, `ctrl @` /
`negctrl @` / `inv @` modifiers, `stdgates.inc` base gates, intrinsic gate
definitions for the bundle/memory ops per geometry, `// bc:` comments for
allocation structure. QIR and Quil (milestone 5) lower from the same IR;
neither is an import format here.

## Milestones

1. **Plan, loader/check, action IR + lowering, OpenQASM 3, differential
   harness** (this milestone; contents below).
2. **Oracle bounded specialisation**: literal §5.2/§3.6 construction into the
   IR, `oracle_call`/`qinplace` inside actions, classical-argument and budget
   specialisation, differential harness against `run-oracle` (F_B on basis
   inputs, full interface tuples); the full static checker (suite F scope:
   §6 provenance/windows, §5.6 borrow frames, §5.5 matched cleanup,
   disjointness including control context, width/operand schemas, bounds
   lists) so `check` agrees with the reference beyond identity.
3. **Placement and scheduling**: liveness reuse, target descriptions (§8),
   gate schemes and ε matching, §11 schedule consumption, frame descriptors.
4. **Proof-carrying output**: §12 certificate generation, §13 coverage
   entries for anything trapped, composition at link time.
5. **QIR and Quil exporters**; §5.7 structure-recovery optimisations under T6
   differential gates.
6. **Scale**: wide-bitvector datapaths without materialising per-bit nodes
   where avoidable, internal branch-enumeration checking for permutation
   segments, demo-ladder circuit artefacts (rungs 1–2 exhaustive both-executor
   checks; rung 4/7/8 oracle circuits at simulation sizes).

## Milestone 1 contents

- This plan; `SPEC_VERSIONS.toml` moved to ISA/object/launch/sched/cert/
  coverage/target 0.8.
- Loader for ISA 0.8 objects sufficient to read Q-tier actions (and to hash
  every section of every fixture), plus `check` agreeing with
  `bennett-isa-ref check` — same verdict, same `object_hash`, same per-symbol
  interface/closure hashes — on every §16.3 example and every fixture under
  `../bennett-isa-spec/reference-interpreter/tests/fixtures/`. Scope limit,
  stated plainly: milestone-1 `check` is parse + schema/signature consistency
  + identity verification; the §6/§5.6/§5.5 static disciplines are milestone
  2, so agreement is claimed (and tested) on the fixture/example corpus, all
  of which the reference accepts, plus rejection agreement on corrupted-hash
  and corrupted-text mutations of it.
- Hierarchical IR (above) and lowering of Q-tier action bodies. Every §4 Q
  row is either implemented or fails with `unimplemented: <row> (milestone
  N)`: milestone 2 gets `oracle_call`, `adj oracle_call`, `qinplace`, and any
  angle/count/classical argument that is not a literal (specialisation
  proper). Everything else — gates, alloc/borrow/free/return, copy/XOR/
  exchange, regions and quantum memory, predication, `qif`/`qloop`/while,
  `action_call`, `adj action_call`, `ctl` — is implemented.
- OpenQASM 3 exporter from the IR and a round-trip test: export, re-parse
  with our own parser, same IR.
- Differential harness: for every `action`/`perm_action` in the fixtures and
  the §16.3 examples (discovered from the objects, never a hard-coded list),
  lower → export → re-parse → flatten → simulate with the sparse simulator in
  `tests/` on every basis input of the interface, and compare the full output
  state against `bennett-isa-ref run-action --basis`. Any divergence fails
  the test printing both states. Workspace cleanliness is asserted on every
  component (borrowed wires start in |0⟩, as in the reference CLI).

## ISA 0.9 touch points (drafted, not accepted — build on 0.8)

From `../bennett-isa-spec/spec/isa-0.9.PROGRESS.md`; none of these change
milestone 1, each names where it lands:

- Version strings become 0.9 everywhere: the loader version-gates the header
  and rejects non-0.8 objects loudly today; accepting 0.9 is a deliberate
  future change, not a default.
- Hashing clarification E3 (direct vs cyclic SCC hashes) is already the
  reading implemented here and is pinned by the §16.3 vectors.
- Index-space SCC ordering E2 and the effective cause of `fault 0` E14 are
  milestone-2 step-machine/commit concerns; the stricter const-literal range
  and call-spelling rules are milestone-2 checker/parser concerns.
- `qinplace` gains `(ins..) (mem..) (args..)` (K4): milestone-2 lowering must
  re-read the accepted shape before implementing the row.
- `budget.mint` (K3) and `qreg[*]` (K1/K2) are CC/R-tier; they reach this
  crate only as already-minted budget tokens on `oracle_call`, unchanged.
- New §16.3 examples 3–9 (including a positive borrow-frame vector) will be
  picked up automatically: both the check-agreement test and the differential
  harness discover objects and actions instead of listing them.
