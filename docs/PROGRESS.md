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
  `check` accepts all 13 fixtures (`status:"ok"`); `run-action --basis` bit
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

## Status

- [x] M1.1 `docs/PLAN.md`
- [ ] M1.2 loader + `check` agreement
- [ ] M1.3 IR + action lowering
- [ ] M1.4 OpenQASM 3 export + round-trip
- [ ] M1.5 differential harness vs `run-action`

Divergences: none yet.
