# bennett-circuits

A bounded circuit specialiser with hierarchical circuit IR, clean-workspace
placement, target resource accounting, proof-carrying output, and OpenQASM 3,
QIR and Quil exporters. The Rust crate has no external crate dependencies.

Workspace handoff: [bennett/HANDOFF.md](../bennett/HANDOFF.md).

Source objects and targets use ISA 0.8. Typed frame and claim metadata requires
explicit `--isa 0.9-draft` opt-in; this does not silently migrate source objects
to ISA 0.9. [SPEC_VERSIONS.toml](SPEC_VERSIONS.toml) records that boundary.
Remaining contract/adoption work stays in Beads. [docs/quality.md](docs/quality.md)
records measured test time, coverage, independent oracles and the quality ratchet.

## Prepare the pinned test oracle

Every sibling read resolves the exact `bennett-isa-spec` commit in `gate.toml`,
through `BENNETT_GATE_LAYOUT` or `.tmp/gate/cache`. Building a live sibling does
not prepare these tests. From the main circuits checkout, materialise/build the
pin with the pinned gate tool (verification only):

```sh
tool_sha=$(sed -n 's/^tool_sha = "\([0-9a-f]*\)"$/\1/p' gate.toml)
mkdir -p .tmp/gate/tool
git -C ../bennett show "$tool_sha:tools/gate_run.py" > ".tmp/gate/tool/gate_run-$tool_sha.py"
python3 ".tmp/gate/tool/gate_run-$tool_sha.py" verify --repo "$PWD" --sha HEAD
```

A worktree whose gate cache is a symlink must run verification against the main
checkout with `--sha <worktree commit>`, rather than `--repo <worktree>`, because
the gate sandbox cannot follow that cache symlink. Tests invoke the reference
interpreter only through its published CLI; no sibling code is imported.

External syntax checks require LLVM 18 (`llvm-as-18`, `opt-18`) and `uv`.
Warm the pinned Python parser packages once:

```sh
uv run --no-project --with 'openqasm3[parser]==1.0.1' --with 'quil==0.37.3' \
  --with 'antlr4-python3-runtime==4.13.2' --with 'numpy==2.5.4' -- python3 -c pass
```

Tests use those parsers offline and fail if validators are missing. Python with
`tomllib` is also required by the pin resolver and quality checks.

## Local verification

Run headless with project-local artifacts:

```sh
unset DISPLAY WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS BROWSER
export TMPDIR="$PWD/.tmp/local/tmp"
export CARGO_TARGET_DIR="$PWD/.tmp/local/target"
mkdir -p "$TMPDIR"
cargo test --offline -j3 -- --test-threads=4
cargo fmt --check
cargo clippy --offline -j3 --all-targets -- -D warnings
python3 -I scripts/quality_metrics.py
```

The default test run excludes two large oracle tests. The local push gate runs
`cargo test --offline -j3 --test oracles -- --ignored --test-threads=1` separately;
allow approximately 7 GB peak memory. Symbolic whole-object loop validation is
retained as an explicitly ignored promised test. Run its currently red witness
with `cargo test --offline -j3 --test integration promised_6kc_ -- --ignored`
(contract bead `bennett-circuits-6kc`).
`scripts/run-conformance` runs the pinned published conformance harness; staged
unavailable probes are reported, not counted as passes.

## Interfaces

`check` and `hash` validate object/proof obligations before reporting identity.
Unresolved imports report pending obligations. `link` resolves and validates a
closure registry. `certificate` emits certificate JSON; `certify-object` attaches
it to an object. Draft metadata requires the explicit version flag above.

`export-{qasm,qir,quil} <object> <region> [--arg value]...` emits executable
circuits. Oracle regions additionally require `--budget B[,heap,stack,depth]`.
Classical arguments are positional integers, `f64:value`, or
`token:B,heap,stack,depth`. `place`, `place-frame` and `place-qasm` add a target
JSON argument; `check-target` validates and hashes canonical target bytes.

`run-action`, `run-oracle`, `run-cc`, `reverse`, `step-check` and `run-program`
provide execution diagnostics. Exact sparse action amplitudes are displayed as
binary64 JSON; classical quantum diagnostics use binary64 arithmetic. Diagnostics
are not numerical/channel certificates. Scheduled execution, context-complete
replay, heterogeneous frame interchange and version adoption remain required
work, with clause-specific dependencies recorded in Beads.

Library entry points live in `lower`, `oracle`, `placement`, `certificate`,
`linking`, `qasm`, `qir` and `quil`. See [docs/PLAN.md](docs/PLAN.md) for design
context and dated design/review documents for historical evidence.
