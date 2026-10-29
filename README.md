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

Milestone 2 is **in progress**, not complete. Oracle construction and action
specialisation are implemented. qinplace compilation exhaustively checks
fault freedom and both inverse compositions under the actual token, with
streaming enumeration and no sampling. ISA certificate serialization is M4;
verified incoming library-certificate support remains M2 checker work.

`check` now validates schemas, widths, cleanup windows and translation-lemma
borrow frames in addition to identity. It agrees on the 14 fixture/example
objects, three corruption variants and 31 Suite F check cases. General
symbolic parameterized-action validation and CFG analyses still have explicit
M2 errors. See HANDOFF.md and docs/PROGRESS.md for the exact gaps; corpus
agreement does not claim a complete static checker.

Harnesses: 19 actions / 930 basis comparisons and 59 oracle occurrences / 488
basis comparisons, all agree. Small oracle interfaces (<=10 bits) are
exhaustive; wider oracle interfaces use four explicit full-tuple patterns.
These tests are evidence, not certificates. `scripts/run-conformance` runs
Suite F using the published harness with all artifacts confined to this repo:
33 pass, 0 fail, 2 unavailable (exit 2, not success).
