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
`--budget`, `--arg`, `--mem`). `run-action` supports all exported gate
families on basis input (`--basis` or `--mut`/`--in`/`--mem`), including
`--arg` and `--adjoint`. Its sparse amplitudes are exact symbolic values;
JSON displays binary64 values. Action `reverse` composes the circuit with
its adjoint and checks exact restoration including phase. These are
execution diagnostics, not numerical/channel certificates. Oracle basis
execution has a Boolean fast path; explicit small budgets avoid large
literal circuit materialisation.

M2 construction and specialisation are implemented. The remaining general
proof obligations have explicit decision beads rather than another milestone
deferral; see [the disposition note](docs/design/2026-10-30-m2-open-proofs.md).
pk3/pk3.2 are closed by the requested disposition process, not as a complete
static-checker claim. Unsupported proofs still fail loudly naming their bead.

M3 placement validates before clean workspace reuse. `placement::place_on_target`
places a hierarchical action; `placement::place_oracle_on_target` additionally
takes its fixed budget. Interfaces, dirty source borrows and local regions
remain dedicated. Concrete classes resolve against target capacities; native
gates, non-nesting schemes, scheme ancillae and QRAM capability drive exact
sequential resource accounting. The target's expressions supply every cost.

`check-target <target.json>` validates canonical target bytes and prints its
hash. `place <object> <region> <target.json> [--budget ...] [--arg ...]`
prints placement metadata; `place-qasm` with those arguments emits executable
placed QASM with the source hierarchy/certificate, bindings, specialisation
request, logical map, timing, resources and obligations attached. This does
not issue a new §12 certificate. QIR/Quil remain M5. The IR comment format is
now `bc-ir-2`, which preserves own versus effective controls.

Supplied schedules refuse with **b-isa-spec-8r4** until the bound library is
published; frame interchange stays **4fa**. Default placement is exact.
Topology/routing input needs its missing §8 schema (**b4k**); `eps_decade`
indices need their missing definition (**v44**). Both errors name their bead.
See the latest PLAN/PROGRESS sections for publication boundaries.

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
