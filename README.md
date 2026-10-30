# bennett-circuits

The bounded circuit specialiser: hierarchical circuit IR, placement, proof-carrying output, and exporters (OpenQASM 3, QIR, Quil).

Depends (by published spec, not by shared code) on: bennett-isa-spec and bennett-sched.

Part of the Bennett reboot workspace. ISA 0.8 action circuits and OpenQASM 3 are implemented; see HANDOFF.md and docs/PLAN.md.


Std-only Rust crate. Local gates (headless, project-local `TMPDIR`):

```sh
cargo test --offline
cargo fmt --check
cargo clippy --offline --all-targets -- -D warnings
```

The differential tests require the already-built sibling
`bennett-isa-spec/reference-interpreter/target/debug/bennett-isa-ref` and its
published spec/fixtures. The reference is invoked only through its CLI.

Commands: `check <object>`, `hash <object>`,
`export-qasm <object> <region> [--budget B[,heap,stack,depth]] [--arg value]...`.
Oracle export requires a fixed budget. Classical arguments are positional
(`--arg 3`, `--arg f64:0.5`, `--arg token:10,0,0,0`). The library APIs are
`oracle::specialise` and `lower::lower_program_with_args`.

`run-oracle` executes the compiled circuit on basis inputs (`--input`,
`--budget`, `--arg`, `--mem`); `run-action` supports permutation circuits with
`--basis` or `--mut`/`--in`/`--mem`, `--arg`, and `--adjoint`. These diagnostic
CLI commands use exact Boolean basis evolution. Non-permutation CLI execution
reports an explicit staged error; the independent test simulator supports all
exported gate families exactly. Pass an explicit small budget for circuit
execution; the reference-compatible default of one million steps is expensive
when materialised as a literal circuit.

M2 construction and specialisation are implemented. The remaining general
proof obligations have explicit decision beads rather than another milestone
deferral; see [the disposition note](docs/design/2026-10-30-m2-open-proofs.md).
pk3/pk3.2 are closed by the requested disposition process, not as a complete
static-checker claim. Unsupported proofs still fail loudly naming their bead.

M3 has a first placement slice: `placement::place_action(&object, action,
&args)` returns a flat circuit, logical-to-physical mapping and live intervals.
It validates cleanliness before reusing clean workspace with disjoint gate
lifetimes and the same requested class. Interfaces, dirty borrowed wires and
local-region wires stay dedicated. `any` remains unresolved until target
assignment. This API rejects supplied schedules naming **b-isa-spec-8r4**;
full target assignment and schedule consumption remain M3 work.

`check` validates schemas, widths, cleanup windows, matched invocation
multiplicities, translation-lemma borrow frames and the documented incoming
certificate subset in addition to identity. General symbolic/opaque workspace
proofs and broader certificate replay retain explicit named errors. See
HANDOFF.md and docs/PROGRESS.md for exact coverage.

Harnesses: 19 actions / 930 basis comparisons and 59 oracle occurrences / 488
basis comparisons, all agree. Small oracle interfaces (<=10 bits) are
exhaustive; wider oracle interfaces use four explicit full-tuple patterns.
These tests are evidence, not certificates. `scripts/run-conformance` uses the published harness with all artifacts
confined to this repo. Select `--suite A`, `--suite B`, or `--suite F` (default).
Latest: A 21/0/0, B 35/0/0, F 33/0/2 (pass/fail/unavailable); F exits 2
because its linking and frame-descriptor probes are unavailable.

`run-cc <object> <entry> [--args ...] [--entropy hex] [--draws ...]
[--replies file]` runs the independent CC/R diagnostic machine. `reverse`
executes and reverses its recorded deltas, refusing measured runs. For oracle
entries, `reverse` accepts `--input`, `--budget`, optional `--steps`, and the
run-oracle arguments, and verifies compiled prefix restoration. `step-check`
compares supported direct row semantics against the circuit on the exhaustive
<=16-bit free input/memory domain. Unsupported direct rows and large-domain
sampling fail explicitly. These diagnostic commands are not certificate
claims; CC/R quantum diagnostic amplitudes use binary64 arithmetic.
