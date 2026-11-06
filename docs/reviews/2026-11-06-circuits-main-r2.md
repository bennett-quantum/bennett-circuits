# Circuits main, review round 2

**Verdict: CLEAR. P0/P1: none.** R1 P1-1, **bennett-circuits-c9l**, is fixed; its closure was independently verified and annotated in Beads.

Reviewed fix range `4718365077ca70ee6bc42008e336ae90c346b9d4..34a8b6ea586112ac77fa40eccc5eccf06478fc76`; exact reviewed head **34a8b6ea586112ac77fa40eccc5eccf06478fc76**. Also inventoried `main..job/pin-circuits` (`main=e2910cb1dd6215dda8f0ba959fe2c3f22583f816`) against the pin review CLEAR recorded by `23c1252d7ceb3b18e98962f76652617676ce736f`. Since that commit, only the r1 review document, one main-merge Beads export addition, and this forwarding-test fix have changed. No other unreviewed implementation is present; `git diff main...HEAD -- src` is empty.

**P1-1 reproduction:** ran `timeout 180s python3 -` with the in-memory Rust harness documented in the r1 review, compiling `src/classical.rs` and the integration tests against this job's built rlib. The identical mutation replaces the single `self.n(a, &Operand::Name(n.clone()))?.to_string(),` expression with `"0".to_string(),`. Four probes, each compiled with `rustc --edition=2024 --test -` and run with `region_call --test-threads=4`, gave:

- Previous tests, read via `git show main:tests/classical.rs`, plus mutant: **2 passed**, exit 0 (reproduces r1).
- Head tests plus unmodified executor: **2 passed**, exit 0.
- Head tests plus mutant: **2 failed**, exit 101.
- Head tests restricted in memory to `adjoint=true`, plus mutant: **2 failed**, exit 101.

Both head mutant cases fail loudly at the specialised loop-bound check (`count 0` differs from declared bounds), before measurement. No source file was edited. Unlike the old unused-parameter action, the new action consumes both arguments. Its expected outputs are independent literals: on little-endian wires, `x t[0]; cx t[0] t[1]` cycles `0→3→2→1→0`; one iteration followed by three `x t[1]` gives `0→3→1`. The standalone adjoint reverses the order and gates, giving `0→2→3`. Checked against the published ISA 0.8 §§3.3, 4.6–4.7 and 5.3 at gate pin `ecaaac060c44e317d1181345fa6b8e38c096c099`.

`materialized_object` accurately names the fixture boundary. The signed-constant test retains `Object::check`; the forwarding fixtures exercise the executor and argument specialisation directly. The pre-existing symbolic-loop whole-object checker refusal at `src/checker.rs:1449` is unchanged and already tracked by **bennett-circuits-6kc**; this review makes no end-to-end checker-support claim.

Focused verification, offline and headless with this job's TMPDIR/target (no full repo or sibling gate):

- `timeout 240s cargo test --offline -j3 --test classical -- --test-threads=4`: **3 passed**.
- `timeout 180s cargo test --offline -j3 --test proof_output legacy_certificate -- --test-threads=4`: **3 passed**.
- `timeout 180s cargo test --offline -j3 --test target zero_time_scheme_must_keep_distinct_ancillae -- --test-threads=4`: **1 passed**.
- `timeout 180s cargo test --offline -j3 --test diagnostics -- --test-threads=4`: **6 passed**.
- `timeout 180s cargo test --offline -j3 --test static_checker suite_f_check_verdicts_and_rules -- --test-threads=4`: **1 passed**.
- `timeout 120s cargo test --offline -j3 --test static_checker borrow_certificate -- --test-threads=4`: **2 passed**.
- `timeout 120s cargo test --offline -j3 --test proof_output legacy_numerical_claims_cannot_bypass_the_expansion_library_gate -- --test-threads=4`: **1 passed**.
- `timeout 60s rustfmt --edition 2024 --check tests/classical.rs`, `git diff --check 4718365..34a8b6e`, and `git diff --check main...HEAD`: clean.

**P2+ nits — bennett-circuits-mur:** carry forward the existing `8jp`/`56n` cleanup: missing quality ratchet, retained HANDOFF/PROGRESS and README references, missing pin-materialisation instructions, and previously recorded long executor/placement functions; no new nit in the fix.

Coverage: read closely every fix hunk, the full classical tests, region argument binding, lowering/substitution/bounds, the relevant checker boundary and pinned spec clauses; inventoried all post-CLEAR paths and commits, skimmed inherited review/Beads metadata. Checked every COMMON Anti-slop rule (dispatch/state, defaults, predicates/constants, altitude/wrappers, identifiers/litter, and test sensitivity/oracle independence); no new blocking violation or scope narrowing. The original referenced `briefs/circuits-main.md` remains absent; the available `circuits-main-r2.md` was read in full and its zero-mutation requirement verified.

Provenance: implementer gpt-6.1-sol; reviewer gpt-6.1-sol, fresh session, D9. No code fixes.
