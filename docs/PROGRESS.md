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
- [ ] M1.5 differential harness vs `run-action`

Divergences: none yet.

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
