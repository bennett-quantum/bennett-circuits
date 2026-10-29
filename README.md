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
`export-qasm <object> <action>`. Export verifies stored identity first.
Oracle calls, `qinplace` and non-literal specialisation report explicit
milestone-2 errors. The full checker is staged in M2; current `check` claims
agreement on the fixture/spec corpus, not complete suite-F conformance.

Harness: 19 actions, 930 exhaustive basis comparisons, zero divergences;
exact support/workspace checks and phase-sensitive binary64 amplitude
comparison at 2e-12. This is test evidence, not a certificate.
