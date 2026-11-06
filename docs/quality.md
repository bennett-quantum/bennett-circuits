# Quality evidence

## Contract audit

All source contracts were read through `scripts/pinned_sibling.py` at
`ecaaac060c44e317d1181345fa6b8e38c096c099`. Supported source/target versions
remain 0.8; bundled 0.9 text supplies an explicit draft proof mode, not implicit
version migration. The open contract and adoption tasks have clause-specific
notes in Beads (frame §9.4; certificate §§12.1–12.3; target §8; library §11.1).
The symbolic-loop experiment is not adopted: §4.8 lists wires, bundles,
regions and literals as Q instruction operands, §5.3 fixes text-order bounds,
and the pinned reference rejects `qloop n` with §16.1 (canonical unsigned INT).
The runtime extension binds integer counts before lowering; whole-object
acceptance needs an explicit parameter/bound contract. A named ignored promised
test retains the requested capability. Evidence: `symbolic-red.log` and
`symbolic-green.log` in the ignored job directory; the latter exposes two
reference-contract regressions from the experimental extension.

Further measurements and the quality baseline are recorded below as completed.

## Default test budget

`CARGO_TARGET_DIR=.tmp/quality-circuits/target /usr/bin/time -f
"wall_seconds=%e peak_kb=%M" timeout 180 cargo test --offline -j3 --
--test-threads=4` passed in **22.00 s**, peak **128280 KiB**, after a separate
cold build (141 s). Test profile keeps optimisation, assertions and overflow
checks, with line-table debug information. Two integration binaries replace
15 independently linked binaries. Two large oracle tests are explicitly
ignored in the default run; `gate.toml` runs them with `--ignored` and one
thread. Their approximately 7 GB historical peak is a resource requirement,
not a reduced semantic domain. The symbolic-object promised test is separate.

## Coverage

`CARGO_TARGET_DIR=.tmp/quality-circuits/target timeout 900 cargo llvm-cov
--offline -j3 --summary-only -- --test-threads=4` passed: **85.08% lines**
(18,021 lines, 2,689 missed), **77.20% functions**, **81.66% regions**.
Siblings used the gate pin. This default-suite measurement excludes the two
ignored large oracle tests and the explicitly promised symbolic-object test.
It includes all maintained default test binaries; there is no coverage threshold.

The five lowest line figures and remaining work are:

| Module | Lines | Reason for remaining uncovered paths |
|---|---:|---|
| main.rs | 65.29% | CLI malformed-option combinations and schedule/target-launch paths exceed the current diagnostic command corpus; executable schedule adoption has an explicit library dependency. |
| linking.rs | 72.75% | Transitive conflict, certificate-DAG and malformed closure combinations remain beyond the existing linked-program replay witnesses. |
| ir.rs | 72.79% | IR construction errors and malformed geometry are incompletely enumerated; exporter/differential suites exercise valid circuits exhaustively over their basis domains. |
| step_check.rs | 73.61% | Direct diagnostic row/error variants exceed the small-width row corpus; large-domain diagnostics remain evidence, with explicit errors. |
| target.rs | 76.77% | Malformed expression/template combinations and version-adoption branches are not all exercised; exact resource and placement witnesses cover valid native/scheme/QRAM paths. |
