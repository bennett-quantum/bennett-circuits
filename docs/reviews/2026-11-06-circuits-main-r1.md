# Circuits main, review round 1

**Verdict: BLOCK.** P0: none. P1-1: **bennett-circuits-c9l**.

Reviewed `origin/main..main`: `9ef99d89d2aad182fa412b1f7a39316bd393d54c..e2910cb1dd6215dda8f0ba959fe2c3f22583f816` (all ten commits). Reviewed main head: **e2910cb1dd6215dda8f0ba959fe2c3f22583f816**. Tests ran at `job/pin-circuits` head `1b01f032e8d612f5308667c5f9b0a1f1a8eacd50`; `git diff main..HEAD -- src` is empty. This is a retrospective main review, not a review of the pin-branch diff.

**P1-1 — value forwarding is untested:** `tests/classical.rs:66` and `:97` (implementation `src/classical.rs:387`). Both new CC/R forwarding tests use an action whose `k:i8` parameter is unused: its body always executes `x t`. They detect the former refusal of nonempty argument lists, but cannot detect forwarding the wrong value. An in-memory mutant replacing the argument expression with `"0".to_string()` passed **both tests**, including their adjoint cases. Under the brief's semantics-bearing test-integrity / claimed-behaviour-without-a-test rule, this blocks. Add a callee that actually uses the supplied value and independently derived expectations that reject zeroed/misbound arguments. The finding survives pin HEAD.

Reproduction run: the unmodified command below passed 3/3; this timeout-bounded mutation probe also exited zero with 2/2 passing, without editing any source file:

```python
# Run from the review worktree with its prescribed TMPDIR/CARGO_TARGET_DIR:
# timeout 120s python3 - <<'PY' ... PY
import os, pathlib, subprocess
r = pathlib.Path.cwd()
s = (r / "src/classical.rs").read_text()
n = 'self.n(a, &Operand::Name(n.clone()))?.to_string(),'
assert s.count(n) == 1
s = s.replace(n, '"0".to_string(),')
t = (r / "tests/classical.rs").read_text().replace("    classical,\n", "")
p = '''pub use bennett_circuits::{Error, Result, ir, json, object, oracle, lower, flatten, sha256};
macro_rules! bail { ($($t:tt)*) => { return Err(crate::Error(format!($($t)*))) }; }
pub(crate) use bail;
'''
lib = next((pathlib.Path(os.environ["CARGO_TARGET_DIR"]) / "debug/deps").glob("libbennett_circuits-*.rlib"))
binary = pathlib.Path(os.environ["TMPDIR"]) / "forward-mutant"
subprocess.run(["rustc", "--edition=2024", "--test", "-", "--extern", f"bennett_circuits={lib}", "-L", f"dependency={lib.parent}", "-o", str(binary)], input=p + "mod classical {\n" + s + "\n}\n" + t, text=True, check=True, timeout=90)
subprocess.run([str(binary), "region_call", "--test-threads=4"], check=True, timeout=20)
```

Focused verification (all offline, headless, job-local target/TMPDIR; no repo gate):

- `timeout 240s cargo test --offline -j3 --test classical -- --test-threads=4`: 3 passed.
- `timeout 180s cargo test --offline -j3 --test proof_output legacy_certificate -- --test-threads=4`: 3 passed.
- `timeout 180s cargo test --offline -j3 --test target zero_time_scheme_must_keep_distinct_ancillae -- --test-threads=4`: 1 passed.
- `timeout 180s cargo test --offline -j3 --test diagnostics -- --test-threads=4`: 6 passed.
- `timeout 180s cargo test --offline -j3 --test static_checker suite_f_check_verdicts_and_rules -- --test-threads=4`: 1 passed.
- `timeout 120s cargo test --offline -j3 --test static_checker borrow_certificate -- --test-threads=4`: 2 passed.
- `timeout 120s cargo test --offline -j3 --test proof_output legacy_numerical_claims_cannot_bypass_the_expansion_library_gate -- --test-threads=4`: 1 passed.
- `git diff --check origin/main..main`: clean.

Red-side inspection used `git show origin/main:<path>`: negative constants hit unsigned parsing; CC/R region calls reject nonempty classical arguments; zero-duration ancillae alias within one expansion; default 0.8 certificate replay returns early and accepts the false program relation. These explain failures of the corresponding new regressions before the main changes. Main already contains the fixes; no checkout, worktree or branch switch was made.

**d2fd693 on its own:** no confirmed P0/P1. Declared-version selection preserves implemented 0.8 expectations and asserts load-time refusal of incompatible versions; its foreign-version unit test passed when compiled directly from `git show main:tests/support/corpus.rs` and the main-side diagnostics test into a temporary Rust test binary (`timeout 90s`, `--test-threads=4`). Independent inventory of pin `ecaaac060c44e317d1181345fa6b8e38c096c099` found selected A/B/C/D/G/H cases all 0.8 and F comprising 30 cases at 0.8 plus the 0.7 rejection case. The original adapters still read the live sibling; the later pin branch removes the selector, tally, its unit test and expectation-deselection branches entirely, restoring published expectations against the pin. No d2fd693 selection/tolerance code survives pin HEAD. Ratified A3 requires explicit draft opt-in for positional tables and resolved standalone invocation encodings; those gates are not treated as new contract decisions here.

**P2+ nits — bennett-circuits-8jp:** plan items 9–11 checked: no quality-baseline ratchet in `gate.toml`; `HANDOFF.md` remains with README references at lines 7/69 and tracked `docs/PROGRESS.md`; touched semantics files retain `place_target_program` at 303 lines (`src/placement.rs:249`) and `execute` at 538 (`src/classical.rs:445`); `tests/support/corpus.rs:1` adds bead identifiers to a source comment (removed at pin HEAD). No tracked pycache/log/lock/backup additions found.

Coverage: read closely all production/test hunks, surrounding replay/execution/placement/metadata paths, relevant published 0.8 clauses and the ratified A3 input contract; metadata-only bead/worklog/review-document changes inventoried, not relied upon as evidence. Checked every COMMON Anti-slop rule; no other confirmed blocker. Limitation: the referenced orchestrator `briefs/circuits-main.md` is absent (searched `.tmp/orchestrator`); scope was retained as the full stated main range.

Provenance: implementer gpt-6.1-sol; reviewer gpt-6.1-sol, fresh session, D9. No code fixes.
