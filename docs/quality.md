# Quality evidence

Final default verification: **210 Rust tests passed**, three explicitly ignored
(two slow oracle tests and one contract-dependent promised test), **8.22 s**
wall time. Source line coverage: **85.15%**. The commands and limits are below.

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
"wall_seconds=%e peak_kb=%M" timeout 900 cargo test -j3 --
--test-threads=4` passed in **8.22 s**, peak **127272 KiB**, with existing
build artifacts. Test profile keeps optimisation, assertions and overflow
checks, with line-table debug information. Two integration binaries replace
15 independently linked binaries. Two large oracle tests are explicitly
ignored in the default run; `gate.toml` runs them with `--ignored` and one
thread. Their approximately 7 GB historical peak is a resource requirement,
not a reduced semantic domain. The symbolic-object promised test is separate.

## Coverage

`CARGO_TARGET_DIR=.tmp/quality-circuits/target timeout 900 cargo llvm-cov
--offline -j3 --summary-only -- --test-threads=4` passed: **85.15% lines**
(18,565 lines, 2,757 missed), **77.83% functions**, **82.03% regions**.
Siblings used the gate pin. This default-suite measurement excludes the two
ignored large oracle tests and the explicitly promised symbolic-object test.
It includes all maintained default test binaries; there is no coverage threshold.

The five lowest line figures and remaining work are:

| Module | Lines | Reason for remaining uncovered paths |
|---|---:|---|
| main.rs | 60.06% | CLI malformed-option combinations and schedule/target-launch paths exceed the current diagnostic command corpus; executable schedule adoption has an explicit library dependency. |
| linking.rs | 73.55% | Transitive conflict, certificate-DAG and malformed closure combinations remain beyond the existing linked-program replay witnesses. |
| ir.rs | 72.79% | IR construction errors and malformed geometry are incompletely enumerated; exporter/differential suites exercise valid circuits exhaustively over their basis domains. |
| step_check.rs | 72.25% | Direct diagnostic row/error variants exceed the small-width row corpus; large-domain diagnostics remain evidence, with explicit errors. |
| target.rs | 76.77% | Malformed expression/template combinations and version-adoption branches are not all exercised; exact resource and placement witnesses cover valid native/scheme/QRAM paths. |

## Assertion and oracle audit

The audited fourteen tests now either assert reported proof status, declared
borrow widths `[2, 1]`/distinct lifetime positions, the emitted expected-program
binding plus certificate replay, nonempty executed conformance counts, or
exported wire geometry in addition to the existing exhaustive exact-state
helper assertions. The while-selector test also mutates a protected selector
and requires rejection. The dirty-workspace simulator test intentionally uses
`#[should_panic]`, which is an assertion by the test runner.
`timeout 900 cargo test --offline -j3 --test integration -- --test-threads=4`
passed 183 tests (one promised ignored) in 29.91 s. Specification examples are
discovered from the pinned text without a fixed count; a three-example witness
checks discovery. Expected borrow widths and tuple scope come from the source
signatures/lifetimes, not captured output.

## Named evaluation phases

The symbolic-history evaluator now has explicit state initialisation, instruction
family handlers and exit validation. `evaluate_body` shrank from 1,157 to
125 lines; the longest new handler is invocation replay (280 lines).
Sixteen unit witnesses cover preparation, initialisation, each instruction family
and exit checks. Placement class assignment is a separate peak-liveness phase
with a hand-derived overlapping/nonoverlapping capacity witness.
`timeout 900 cargo test --offline -j3 --lib --test integration -- --test-threads=4`
passed 31 unit tests and 173 integration tests (one promised ignored), in
0.01 s and 20.34 s. Shared integration support is compiled once; duplicate
helper test copies account for the reduced integration count.
`timeout 180 cargo clippy --offline -j3 --all-targets -- -D warnings` passed.
Seventy CLI status/stdout/stderr triplets (all pinned fixture hashes and action
QASM/QIR/Quil exports, including failures) were byte-identical before/after.
The exhaustive pinned reference action differential also passed.

Remaining semantic functions over 300 lines have owner **circuits implementation
stream**, tracked as `bennett-circuits-so2`: `classical::execute` (538), checker
`schema` (329), `static_parameter_action` (306), `classical_schema` (481), oracle
`row` (461) and `specialise_impl` (364). They need opcode/transfer/build phases
with independent witnesses; this checkpoint does not claim those splits done.

## External exporters

QIR requires both `llvm-as-18` and `opt-18`; missing executables panic with
`required external validator <tool>`. A named missing-tool regression checks
this failure, and an empty-PATH reproduction of the old test demonstrated its
incorrect success before the fix. QASM and Quil now pass independent syntax
parsers (`openqasm3` 1.0.1 / ANTLR 4.13.2, `quil` 0.37.3 / NumPy 2.5.4),
including all exported gate families, modifiers and subnormal-angle spelling.
Each parser also rejects a malformed control input. These prove grammar
acceptance; the exhaustive ISA CLI differential remains the semantic oracle.
`timeout 900 cargo test --offline -j3 --test integration exporters:: --
--test-threads=4` passed all 8 exporter tests in 0.94 s.

Parser prerequisites are cached once with:
`uv run --no-project --with "openqasm3[parser]==1.0.1" --with "quil==0.37.3"
--with "antlr4-python3-runtime==4.13.2" --with "numpy==2.5.4" -- python3 -c pass`.
Tests then use `uv run --offline`; a missing parser/tool fails explicitly.

## Product and repository hygiene

Forty-eight bead identifier occurrences and numeric milestone labels were
removed from source/comments/diagnostics. Errors retain their construct,
specification clauses and published `E_` codes; tests assert these stable
contracts. The published `bennett-circuits-placement-1` format identifier is
preserved. CLI launch, target checking, placement and export dispatch now have
separate readable handlers. `SPEC_VERSIONS.toml` says partial 0.8 support plus
explicit draft metadata, rather than the stale first-milestone status.
Tracked `docs/PROGRESS.md` is retired; current progress stays ignored in `.tmp`.

The only remaining tracked litter is `.beads/hooks/pre-push.backup`, outside
this job ownership. Removal is assigned to the orchestrator in
`bennett-circuits-8n7`; the ratchet records that existing path and rejects any
additional litter. Ignore rules cover pycache, bytecode, logs, Beads LOCK,
backups and scratch progress. There are no tracked artifacts added by this job.

## Quality ratchet

`python3 -I scripts/quality_metrics.py --measure` records the baseline below.
`python3 -I scripts/quality_metrics.py` is the single quality check in
`gate.toml`; `--self-test` exercises lexical handling and a mutation for every
ratcheted dimension, and runs during the default integration suite.

Function lengths include source unit tests and nested named functions. The
complexity mass is a lexical proxy: branch/loop/match/Boolean control tokens
weighted by brace nesting inside functions (nested functions are counted in
their enclosing body as well). It is deterministic, not a semantic cognitive
complexity analysis. The gate rejects increases in non-short length buckets,
per-file maximum length, per-file complexity mass, trivial forwarding wrappers,
process identifiers or newly tracked litter paths. Useful short functions can
be added. No universal numeric limit or coverage threshold is introduced;
explicit baseline changes require reviewing the new measurement.

Signature scanning now balances parentheses, brackets, angle brackets and const
expression braces before treating a semicolon as a declaration terminator;
return arrows are distinct from closing angle brackets. The original measurement
omitted three existing array-signature functions: `sha256::sha256` (68 lines,
complexity mass 9) and `certificate::obj` / `classical::obj` (three lines and one
forwarding wrapper each). The corrected baseline therefore changes the short
function count from 400 to 402, the 31–100-line count from 131 to 132, the
`src/sha256.rs` maximum from 15 to 68 and complexity mass from 1 to 10, and the
wrapper count from 102 to 104. These are newly measured existing functions;
the ratchet comparisons are unchanged.

An inventory-level regression adds 320 conditional lines to the real `sha256`
function in memory. It failed before the scanner correction because
`regressions()` returned an empty list; it now rejects both length-bucket and
complexity growth. Seven paired signature/declaration cases cover arrays in
arguments, returns, generic bounds, function pointers and const expressions,
while keeping trait declarations excluded. Malformed signatures fail explicitly.

Corresponding hygiene greps are `rg -n
"bennett-circuits-[a-z0-9]{3}\b|b-isa-spec-[a-z0-9]{3}\b|milestone[ -][0-9]+"
src` (zero matches) and `git ls-files | rg
"(__pycache__|\.pyc$|\.log$|LOCK$|\.backup$|\.darc|LAST.RUN|PROGRESS)"`
(one existing Beads hook backup; removal owner above). The script also detects
job/shift/round labels and session dates. This is the repo's own baseline.

<!-- quality-baseline -->
```json
{
  "complexity_mass_per_file": {
    "src/amplitude.rs": 78,
    "src/borrow.rs": 87,
    "src/certificate.rs": 584,
    "src/checker.rs": 2923,
    "src/classical.rs": 560,
    "src/coverage.rs": 37,
    "src/export.rs": 18,
    "src/flatten.rs": 93,
    "src/frame.rs": 151,
    "src/history.rs": 756,
    "src/ir.rs": 25,
    "src/ir_codec.rs": 55,
    "src/json.rs": 187,
    "src/lib.rs": 0,
    "src/linking.rs": 296,
    "src/lower.rs": 460,
    "src/main.rs": 409,
    "src/metadata.rs": 478,
    "src/object.rs": 868,
    "src/oracle.rs": 525,
    "src/placement.rs": 268,
    "src/proof.rs": 80,
    "src/qasm.rs": 137,
    "src/qir.rs": 166,
    "src/quil.rs": 43,
    "src/selectors.rs": 61,
    "src/sha256.rs": 10,
    "src/simulation.rs": 94,
    "src/step_check.rs": 239,
    "src/target.rs": 438,
    "src/translation.rs": 111
  },
  "length_distribution": {
    "1-30": 402,
    "101-300": 41,
    "31-100": 132,
    "over-300": 6
  },
  "longest_per_file": {
    "src/amplitude.rs": 39,
    "src/borrow.rs": 116,
    "src/certificate.rs": 277,
    "src/checker.rs": 481,
    "src/classical.rs": 536,
    "src/coverage.rs": 62,
    "src/export.rs": 42,
    "src/flatten.rs": 231,
    "src/frame.rs": 126,
    "src/history.rs": 280,
    "src/ir.rs": 55,
    "src/ir_codec.rs": 75,
    "src/json.rs": 75,
    "src/lib.rs": 3,
    "src/linking.rs": 195,
    "src/lower.rs": 259,
    "src/main.rs": 254,
    "src/metadata.rs": 269,
    "src/object.rs": 139,
    "src/oracle.rs": 460,
    "src/placement.rs": 264,
    "src/proof.rs": 76,
    "src/qasm.rs": 96,
    "src/qir.rs": 230,
    "src/quil.rs": 74,
    "src/selectors.rs": 56,
    "src/sha256.rs": 68,
    "src/simulation.rs": 70,
    "src/step_check.rs": 260,
    "src/target.rs": 268,
    "src/translation.rs": 190
  },
  "process_identifiers": 0,
  "tracked_litter": [
    ".beads/hooks/pre-push.backup"
  ],
  "trivial_wrappers": 104
}
```

Ratchet verification: `timeout 30 python3 -I scripts/quality_metrics.py
--self-test` passed all seven tests, and `timeout 30 python3 -I
scripts/quality_metrics.py` passed the corrected baseline. A direct comparison
also confirmed that the recorded JSON equals the corrected inventory in every
dimension. A temporary Git fixture from the initial verification also
rejected an actual added branch (complexity 79 > 78), a source process identifier
(1 > 0), and a newly tracked `trace.log`; the clean fixture passed. The three
pin/ratchet integration tests passed in 1.57 s. Baseline check passes on this tree.

## Final audit

The completed default command passed 31 library tests, 176 integration tests
and three small oracle/support tests; one promised and two slow tests were
explicitly ignored. The parser/ratchet subprocess tests are included. Running
`timeout 180 cargo test --offline -j3 --test integration promised_6kc_ --
--ignored` selects exactly one test and remains red with §16.1/§5.3, as required
until the symbolic object-binding contract is clarified. Its named switch is
also in the README. The two large oracle tests were retained with their full
semantic domains and are assigned to the orchestrator's separate gate run;
this child did not run the full push or any consumer gate.

Final `cargo fmt --check`, `timeout 180 cargo clippy --offline -j3 --all-targets
-- -D warnings`, `python3 -I scripts/quality_metrics.py`, its seven self-tests,
and `git diff --check` all pass. Code written in this change uses named phases,
explicit errors, hand-derived/independent test expectations, one owner for the
history state, and no process labels in product output. No sibling code was
imported; all sibling test inputs resolve the gate pin. The remaining long-function
owner and out-of-scope backup are recorded above, rather than claimed complete.

The four staged F manifests were re-read at the pin. In particular,
`F-link-conflict` still says there is no CLI link command; current `link` is
implemented. Updating that published probe is an upstream task under
`bennett-circuits-46t`; the job's ignored cross-repo request is queued for the
orchestrator. Existing contract/version/breadth tasks stay open with exact
clause/next-step notes. `bennett-circuits-56n` is closed by the completed pin
instructions, handoff retirement and measured quality check.
