# Brief: bennett-circuits milestone 1 (plan + circuit IR + Q-tier lowering + OpenQASM 3 export, differential)

Repository `/home/tobias/Projects/bennett-workspace/bennett-circuits` (skeleton). You are the sole implementer. It
consumes ISA 0.8 only through the published spec `../bennett-isa-spec/spec/Bennett-ISA-Spec.md` (council-accepted);
never copy or link code from the reference interpreter (CLAUDE.md rule 5). Its binary
`../bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref` (README "Commands", especially `run-action`
on a basis state or state vector, and `check`/`hash`) is your differential oracle through its CLI only.

## Read first, in full
`CLAUDE.md`, `README.md`, `HANDOFF.md`, `SPEC_VERSIONS.toml`; `../bennett/docs/Bennett-NorthStar.md` (the sections the
CLAUDE.md names, plus §4 and §8 demo ladder: know which rungs need circuit export); ISA spec §3, §4 (Q-tier rows and
actions), §6 (checker), §9, §12 (certificates, proof-carrying output), §14.1, §16; `../bennett-isa-spec/HANDOFF.md`
(ISA 0.9 is drafted, not accepted: build on 0.8, note 0.9 touch points in your plan).

## Milestone 1
1. `docs/PLAN.md`: architecture for the whole CLAUDE.md scope (bounded specialiser, hierarchical circuit IR,
   placement, proof-carrying output, exporters OpenQASM 3 / QIR / Quil), milestones in order, milestone 1 contents.
2. Object loader for ISA 0.8 objects (§16) sufficient to read Q-tier actions; `check` agreeing with `bennett-isa-ref
   check` on every spec example and fixture under `../bennett-isa-spec/reference-interpreter/tests/fixtures/`.
3. Hierarchical circuit IR and the lowering of Q-tier actions (every §4 Q row: implement or fail with
   "unimplemented: <row> (milestone N)") into it, preserving hierarchy (calls as subcircuits, uncomputation pairs).
4. OpenQASM 3 exporter from the IR.
5. Differential harness: for every action in the fixtures and spec examples, simulate your exported circuit
   (a small exact state-vector simulator inside `tests/`, std only) on basis inputs and compare against
   `bennett-isa-ref run-action`; any divergence is a failing test printing both. Also a round-trip test: export,
   re-parse the QASM you wrote, same IR.

Red-green; fail fast; no narrowing (rule 10).

## Process
Headless (rule 11), `TMPDIR` under this repo (`.tmp/`), display vars unset. `cargo test --offline`, std only unless
truly needed (say so). `bd init` if `.beads/` is absent, milestone epic + tasks, `bd export -o .beads/issues.jsonl`
before committing `.beads/`. Keep `docs/PROGRESS.md` current. Commit regularly: `git -C /home/tobias/Projects/
bennett-workspace/bennett-circuits commit -- <paths>` (retry on index.lock); do not push; absolute paths only; never
`cd` inside compound commands. Never edit any other repository. Final message: commit hash, PLAN summary, harness
counts (actions diffed, agree / diverge), every divergence with the deciding spec sentence, next milestone.
