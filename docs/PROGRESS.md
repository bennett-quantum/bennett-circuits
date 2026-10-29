# bennett-circuits — progress

Milestone 1 (brief `docs/briefs/2026-10-29-circuits-milestone-1.md`), beads
epic `bennett-circuits-q93` (tasks `.1`–`.5`). ISA 0.8 via the published spec
only; reference binary used solely through its CLI in tests.

## 2026-10-29 — setup

- Read in full: repo `CLAUDE.md`/`README.md`/`HANDOFF.md`/`SPEC_VERSIONS.toml`;
  NorthStar §0, §3–§6, §8; ISA spec §3, §4, §5, §6, §9, §12, §14.1–14.3, §16
  (plus §8/§11 heads for the plan); `bennett-isa-spec/HANDOFF.md`; the
  reference interpreter README (CLI contract). All 13 `.bisa` fixtures read.
- Verified against the oracle binary before designing against it:
  `check` accepts all 12 fixtures (`status:"ok"`); `run-action --basis` bit
  order is interface order (mut bundles, in bundles, mem cells; first char =
  first wire; bundle value little-endian) — confirmed by hand on `flip`
  (basis 0110 → 1110), `image` (0x35 XOR cell 0xad = 152), `while3`,
  `phase` (rz(2π)|1⟩ = −1 + 1.2e−16 i). One `run-action` call costs ~5 ms, so
  exhaustive basis enumeration (~740 calls over 13 actions) is affordable.
- §9.2 hashing reading pinned by §16.3 (and the 0.9 draft's derivation D1b):
  a trivial SCC hashes its envelope directly (`inc8`, `zero`); a cyclic SCC
  member hash is SHA-256 of `<scc_hash hex>:<index>` (`sum`). Interface and
  closure hashes coincide on fully-linked objects (no imports in fixtures).
- `bd init` ran (beads epic + 5 tasks above). Note: `bd init` committed its
  own scaffolding itself (359de7d, including a managed block appended to
  `CLAUDE.md`); `scripts/pre-push` remains the installed gate hook.
- Decisions recorded in `docs/PLAN.md`: milestone-1 `check` scope (parse +
  schema/signature + identity; static disciplines in milestone 2); naive
  static placement; borrowed wires exported against a |0⟩ pool (as
  `run-action`); image gates controlled by an enclosing `ctl` at flatten time
  (§5.3) but not by the body's own predicate stack (§3.5); sparse (basis-keyed)
  simulator in `tests/` because `image` needs 33 wires — dense 2^33 is not an
  option, and basis inputs keep component counts tiny.
- `SPEC_VERSIONS.toml`: skeleton 0.3 pins replaced with 0.8 (the spec this
  milestone implements; sub-versions per ISA spec §8–§13).

## 2026-10-29 — loader + check (M1.2)

- `src/sha256.rs`, `src/json.rs` (JCS subset, integer numbers only),
  `src/object.rs` (§16 parser, §9.1 derived-signature validation, §9.2
  identity with SCC handling, `check`/`hash`), `src/main.rs` CLI.
- `tests/check_agreement.rs` green: verdict + `object_hash` + every
  interface/closure hash agree with `bennett-isa-ref check` on all 12
  fixtures and both §16.3 examples (extracted from the spec text itself);
  the `inc8` envelope matches the §16.3 printed bytes; corrupted hash/body/
  signature mutations are rejected by both.
- Surprise resolved by hash evidence: `classical.bisa`'s `unknown_effect`
  uses an undeclared device (`effect missing.op`). That is legal — §17
  item 3 makes it `FAULT_EFFECT_UNMODELLED` at run time — so its envelope
  simply has no 5c entry for it. First implementation errored; skipping
  undeclared entries reproduces the stored hash exactly.

## Status

- [x] M1.1 `docs/PLAN.md`
- [x] M1.2 loader + `check` agreement
- [x] M1.3 IR + action lowering
- [x] M1.4 OpenQASM 3 export + round-trip
- [x] M1.5 differential harness vs `run-action`

Divergences: none (19 actions / 930 basis comparisons).

## 2026-10-29 — resumed IR/lowering

- Retained and reviewed the prior agent's uncommitted hierarchical IR,
  lowering and naive placement. All 13 fixture actions lower and flatten.
- Red-green regressions fixed call-cycle detection, indexed MCX controls,
  lexical entry-time local regions (including zero-trip loops and forward
  uses), structured predicate-stack balance, and milestone error spelling.
- Nine lowering tests pass; calls and copy/load cleanup pairs remain explicit.
  Oracle specialisation and non-literal parameters remain explicit M2 errors.

## 2026-10-29 — OpenQASM 3

- Hierarchical gate definitions, explicit auxiliary operands, stdgates,
  ctrl/negctrl/inv modifiers, and CLI `export-qasm` with identity checking.
- Versioned `bc-ir-1` comments preserve roles, allocation classes, geometry,
  call hierarchy and copy/load pairs. The IR parser rejects executable text
  that disagrees with the metadata; a separate executable parser ignores the
  IR comment and parses/expands real QASM gate statements for simulation.
- Round trips and independent gate-text flattening agree on all 13 actions;
  executable mutation and CLI export tests pass. Zero-wire identities use
  one inert sentinel qubit because QASM gate declarations need an operand.

## 2026-10-29 — differential harness

- `tests/differential.rs`: discovers all actions in 12 fixtures and 2
  normative spec examples; 13 actions / 740 exhaustive basis comparisons.
  Six generated, hash-checked supplemental actions cover all gate families,
  explicit allocation, memory effects, lexical regions and nested
  controlled/adjoint calls with workspace: 190 further comparisons.
- `tests/support/exact.rs`: independent std-only arbitrary-integer dyadic
  arithmetic and exact symbolic amplitudes over Q(sqrt(2),i) with formal
  exp(i*d), d dyadic radians. `tests/support/sim.rs`: sparse simulator over
  arbitrary wire-count bit vectors, no amplitude pruning. Tests pin exact
  cancellation, tiny nonzero support, phase, and dirty-workspace failure.
- Simulation reads the executable QASM gate text, not the IR metadata.
  Exact support and all workspace are checked; binary64 display amplitudes
  compared with absolute tolerance 2e-12, including global phase. This is
  phase-sensitive differential evidence, not a numerical proof/certificate.
- Results: **19 actions, 930 basis comparisons, 930 agree, 0 diverge**.
  Spec examples have no action rows; both are still discovered, parsed,
  hash-checked, and covered by check agreement. Every divergence: none.
- Whole-scope plan remains in `docs/PLAN.md`; next is M2: literal bounded
  oracle construction, `oracle_call`/`qinplace`, classical specialisation,
  and full static disciplines. ISA 0.9 remains an unaccepted draft.

## Final local gates

- `cargo test --offline`: 30 passed, 0 failed, 0 ignored.
- `cargo fmt --check`: clean.
- `cargo clippy --offline --all-targets -- -D warnings`: clean.
- `git diff --check`: clean. Headless throughout, TMPDIR in `.tmp/`.
- Additional red-green regression: generated while-loop flag names must not
  shadow legal user bundle names. Internal names now use an ISA-inexpressible
  `$` prefix. This resolved a lowering rejection, not an oracle divergence.
- M1 epic and all five child beads closed; next bead `bennett-circuits-pk3`.
  No pushes; no external repository edits or interpreter source imports.

## 2026-10-29 — milestone 2 implementation in progress

- Claimed `bennett-circuits-pk3`; split construction, checker and action
  specialisation into child beads `.1`–`.3`.
- Red: oracle test failed to compile because `oracle::specialise` was absent.
  Green: literal inc8 construction passes B=0/2/3/4 with arbitrary target
  and fault values and exact workspace restoration (144 tuples).
- Added gate-level Boolean expansion, bounded dispatch, assignment/capture
  histories, commit and complete gate-prefix reversal. Expansion scratch is
  retained as private history until reversal; no certificate claim is made.
- Array storage omits addresses that cannot belong to a live allocation
  under the fixed heap budget; dispatch retains actual ISA addresses. Stack
  storage is bounded by the specialised stack limit. This removes no valid
  access: inherent inrange/call flags guard every access.
- Oracle corpus differential coverage, nested rows, full static checker and
  action specialisation remain in progress; milestone 2 is not complete.

## 2026-10-29 — milestone 2 checkpoint (not complete)

- All §4.2 oracle rows now have gate constructions: arithmetic (including
  arbitrary-width signed division), casts, parallel assignments, canonical
  labels and indirect branches, allocator/inherent memory checks, lproc
  capture/push/bind/delivery/counters, nested oracle calls and permutation
  actions. The IR retains hierarchical nested calls. Dispatch, row effects,
  successor XORs, pc commit and dispatch cleanup are ordered as §3.6; allocator
  assignment slots and exchanges follow §4.2 order. Expansion scratch remains
  private history until the complete gate prefix is inverted.
- Reachable-pc sets omit provably inactive row expansions at each unrolled
  step (a conservative CFG overapproximation, including every possible indirect
  destination). This reduced div257 from 18 million gates to about 2.25 million;
  it does not choose a branch from a wire value. Allocation/stack memory is
  limited to addresses that can be live under the fixed budget, with ISA
  numeric addresses retained.
- Oracle differential: **59 discovered oracle occurrences / 488 full-interface
  basis tuples / 488 agree / 0 diverge** across all 12 fixtures and both spec
  objects. Interfaces of <=10 bits are exhaustive; larger interfaces use four
  explicit patterns (all-zero, all-one, alternating, every third bit). Every
  tuple varies targets/faults as well as inputs/memory. These are samples for
  wide domains, not proof. Budgets are body-row-count+6 (sum_db:12), heap capped
  at 32, stack capped at 64, depth 4 where layout exists; nested tokens use B=4.
  Additional inc8 checks cover B=0/2/3/4 and 144 arbitrary target/fault tuples.
- `lower_program_with_args` specialises counts/angles and hierarchical action
  calls. `oracle_call`, `adj oracle_call`, `ctl`, and qinplace's compute/exchange/
  inverse sequence are lowered. Tests cover all 512 controlled-call interface
  tuples, all four qinplace inputs and adjoint, and count/angle call-adjoint
  cancellation. CLI export accepts `--budget` and repeated `--arg` values.
- Static validation now precedes identity for check/hash: lifted row schemas,
  widths/name resolution/tier restrictions/terminators; basic CC/R schemas and
  straight-line qreg handle lifetimes; action cleanup via symbolic Boolean
  algebra/inverse histories, load/copy pairing and cell-version windows;
  translation-lemma borrow frames; bounds, read-only interfaces, effective
  control disjointness and region cycles. **All 14 corpus objects + 3 corrupt
  variants + 31 Suite F check cases agree** (48 verdict comparisons).
- Red/green: width-invalid objects were accepted by hash, the cell-version
  mutation was accepted, allocation slot-order metadata was absent, and CLI
  oracle export rejected budget options. Each regression now passes. Fault
  writes initially lacked their dispatch control; mutual recursion initially
  sorted SCC members by individual hash. Corrected under §3.6: "under at_k";
  "lprocs ordered by closure hash with SCC members in SCC index order".
- `scripts/run-conformance` compiles the published conformance harness unchanged
  with rustc into this repo's .tmp, using a local CARGO_MANIFEST_DIR so the
  harness also writes its run directory here. External sources/cases remain
  read-only. Suite F: **31 pass / 2 fail / 2 unavailable**. Failures are
  F-adjoint-table and F-index-space-flattening because run-action/run-oracle
  CLI commands are absent (the corresponding library circuits are tested).
  Unavailable cases are the harness's linking and frame-descriptor cases.

### Remaining milestone-2 acceptance work

The milestone and checker are **not complete**. Do not close pk3 or claim a
full static checker on corpus agreement. Explicit staged errors remain for
symbolic classical-action validation beyond straight-line interface oracle
calls, §5.5 cleanup across CFG joins/indirect jumps, §3.3 qreg handle analysis
across CFG joins, imported lproc/classical/action schema/body cases, and
certificate-backed borrow frames. qinplace mechanics are implemented, but
its required full-domain inverse/fault-free certificate validation remains.
Metadata nested schemas, provenance through arbitrary calls and selector
immutability still need a complete audit; the current checker is a tested
partial implementation. These are M2 work, not scope removed or moved to M3.

## 2026-10-29 — inverse proofs and conformance execution

- Closed the qinplace safety gap red-green: B=0 and inc2 paired with itself
  were accepted. Compilation now streams every n-bit input, checks exact
  workspace/input restoration, fault freedom and both inverse compositions
  using compiled X/SWAP circuits. The returned internal proof binds the object,
  both closure hashes, width and actual budget. No sample or imported claim is
  trusted; this does not emit an ISA §12 certificate (M4).
- Added compiled basis `run-oracle` with diagnostic active-row observer bits
  (never part of exported oracle interfaces). Observer writes persist while
  the original gate prefix is reversed; their sum reports executed rows,
  excluding HALT padding and counting nested calls as one caller row (§4.2).
  Added exact permutation-basis run-action with adjoint support. The published
  F-adjoint-table and F-index-space-flattening now pass; Suite F is **33 pass /
  0 fail / 2 unavailable**, exit 2. The earlier 31/2/2 checkpoint is superseded.
- Red-green call-boundary regression: duplicate interface bundles and effective
  controls overlapping call operands were accepted when a zero-trip callee
  generated no gates. Disjointness is now checked at the call boundary (§3.3),
  independent of its expansion. Pointer and integer classical arguments are
  range checked, including unused parameters.
- The full M2 checker is still unfinished. qinplace's specialised exhaustive
  proof is implemented; incoming certificate/library validation, general
  symbolic parameterized actions, CFG matched cleanup/handle joins and the
  remaining audit items above stay M2. Do not infer full conformance from the
  finite corpus or move these obligations into M3.

## Latest local verification

- `cargo test --offline -- --nocapture`: **49 passed, 0 failed, 0 ignored**.
  Oracle harness: 59 occurrences / 488 agree / 0 diverge. Existing action
  harness: 19 actions / 930 agree / 0 diverge. All workspace checks exact.
- `cargo fmt --check`, strict Clippy (all targets), `git diff --check`: clean.
- Published conformance harness, Suite F: **33 pass, 0 fail, 2 unavailable**;
  exit 2. Unavailable: F-link-conflict and F-frame-live-ranges, counted by the
  external harness (no execution probe exists).
- No remaining observed semantic divergence, so no deciding-spec disagreement
  to report. Resolved implementation mismatches and their rules are recorded
  above. Corpus agreement and sampled wide domains are evidence only.
- Child beads pk3.1 (oracle construction) and pk3.3 (action specialisation)
  closed. pk3.2 (full checker) and pk3 remain in progress. **Milestone 2 has not
  met its full acceptance criteria**; M3 follows only after those gaps close.
- No pushes, no edits outside this repository, reference interpreter CLI only.

## 2026-10-29 — resumed checker: CFG cleanup and broader citation audit

- Red-green §5.5 worklist analysis now follows direct CFG edges, maintains
  distinct reaching invocation identities across joins and converges on
  balanced loops. Cleanup must have a matching call on every incoming path.
  Writes through another oracle/permutation call invalidate pending input
  versions; recursive lproc memory effects are inspected transitively.
- Explicit M2 errors remain for overlapping invocations at the same cyclic
  call point and interprocedural indirect-jump cleanup. General symbolic
  actions, handle joins, certificate-backed borrow proofs and nested schemas
  are still open. This checkpoint does not complete M2.
- Expanded differential checks to all **42 conformance object files**:
  verdict and diagnostic rule citation agree. Fixed diagnostics for version
  rejection (§1) and illegal lifted opcodes (§4.8). Existing 14 fixture/spec
  objects + 3 corruptions + 31 F verdict comparisons remain covered.
- Three CFG regressions and the expanded corpus test passed (7 checker tests).
  Suite A baseline is **0 pass / 21 fail / 0 unavailable**: run-cc/reverse CLI
  is absent. Suite B baseline is under investigation: literal expansion for
  B=1000 consumed >8 GB on depth-limit before completing; stopped that child
  process to avoid exhausting the host. No expected result was altered.

## 2026-10-29 — resumed checker: handle joins, borrow proofs and schema audit

- Added path-sensitive CC/R schema/handle worklists with block-argument
  renaming, view-parent tracking, balanced loop convergence and checking of
  unreachable blocks. Regressions cover branch-local frees, a consumed handle
  on one incoming path, and a view escaping without its one region call.
  Imported lproc and CC/R calls now use the published §9.1 signature, without
  requiring or reading an implementation body. Imported region bodies and
  their arbitrary-call provenance still need the general symbolic checker.
- Added recursive validation of signatures/imports, devices, layout leaves,
  schedules, coverage, and certificate claims/domains/verification indices,
  producer and composition fields. Unsupported certificate frame-descriptor
  schemas fail explicitly. This is not a completed audit of every schema:
  optional/required-field coverage, frame descriptors, arbitrary-width domain
  metadata and remaining canonical/provenance checks still need review.
- Certificate-backed borrow fallback now accepts bound exhaustive T3 claims
  with `expected.program` naming the actual current closure. It verifies the
  object/root/region/interface bindings and full interface/borrowed/region D,
  then streams *every* basis configuration of each maximal permutation frame
  (including arbitrary non-borrowed frame wires), checking B restoration and
  independence of all other outputs from B. It permits swap/swap-back frames
  outside the translation lemma. Sampled claims, truncated D, wrong bindings,
  and dependent output fail. Expected tables/external programs and structural
  library replay remain explicit M2 errors; no library certificate is trusted.
  ISA certificate generation remains M4.
- Found and fixed two false acceptances in §6.2: H–S–S–H on a clean workspace,
  and H–CX-to-output–H. Local Boolean equality had erased interference and
  entanglement. The checker now tracks `other` values and their quantum uses;
  phases on perm copies still transfer to the declared operation (T5).
  Exact adjoints cancel only across disjoint supports. Window-closing events
  are checked on the original sequence, before that cancellation. The action
  differential harness caught a too-conservative first fix on a phase-kickback
  copy; the final fix preserves its valid cleanup and all 930 comparisons.
- Selector auditing rejects modification even if subsequently undone, and
  protects while-condition dependencies. Indexed disjoint selectors remain
  legal. Three newly materialised, correctly hashed negative objects agree
  with the reference CLI (interference, entanglement, selector modification).
- Basis diagnostics no longer materialise all HALT padding: try bounded
  prefixes and propagate the known basis through the diagnostic gate builder,
  skipping inactive row expansions. Exported circuits retain the full-D,
  fixed-budget construction. A million-step inc8 diagnostic regression passes;
  published B-depth-limit and B-stack-exhaustion now pass without multi-GB
  circuit materialisation. No expected outcome was changed.
- Check comparisons: **93 agree / 0 diverge** = 14 fixture/spec objects +
  3 corruptions + 31 F check invocations + 42 conformance object files +
  3 new witnesses. These categories deliberately include repeated corpus
  contents; this is a comparison count, not 93 distinct semantic programs.
- Published Suite B: **31 pass / 4 fail / 0 unavailable**. Remaining failures:
  B-call-step-reversal, B-prefix-reversal-inc8, B-ret-step-reversal (no oracle
  `reverse` CLI), B-step-check-tiny (no independent `step-check` CLI).
  Suite A remains **0/21/0** (no CC execution/history CLI). Suite F checkpoint
  remains **33/0/2**; final rerun recorded below.

### Current rule coverage and acceptance gaps

§6.1 qalloc/copy/XOR/exchange/load/store/cell versions and concrete gate/call
expansions are checked; oracle and qinplace specialisation remain as before.
§6.2 exact cleanup/window checks now preserve quantum interference and uses;
§6.3 T5 diagonal-use cleanup remains supported. §6.4 effective control
operand disjointness, exact inverse checks and rejection of measurement-based
cleanup realisations are implemented. Full symbolic parameterized actions,
opaque arbitrary-call transfer/provenance, full inherited-context schedule
keys, all incoming library certificates and complete schemas are not done.
§5.5 direct CFG cleanup is implemented, but overlapping dynamic invocations
at cyclic sites and interprocedural indirect jumps remain explicit M2 errors.
§5.6 translation frames and the bound exhaustive-program subset above are
implemented; expected-table/external-library/structural replay remains staged.
The suite A/B CLI obligations above remain acceptance work, not exclusions.
**Milestone 2 remains incomplete. pk3 and pk3.2 must stay open.**
No observed reference disagreement, so no deciding-spec divergence to report.
