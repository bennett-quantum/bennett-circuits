# Pin discipline review, round 1

Verdict: **CLEAR**. P0/P1: none.

Reviewed `main...job/pin-circuits`, base `a6982767cb8e258076366bf81530f5d824c2b602`, commits `5c534a79c6c88dbba0dcff127da0ba390f5a2fe4` through exact head **`9af1cc8eba76bd429465647a830d7359952f630e`**.

The resolver checks the full pinned HEAD, selects the gate layout before the cache, and fails without a live-tree fallback. All twelve Rust readers and `scripts/run-conformance` use it. Only the diagnostic HEAD query consults the live sibling. Removing `d2fd693`'s classifier restores full published expectations; the incompatible-version case retains its explicit refusal and section-citation assertions. The classifier self-test was removed with that deleted mechanism; the ISA-version invariant remains in `tests/pinned_siblings.rs`. No surviving assertion was weakened, case skipped, tolerance widened, or product capability narrowed.

Verification (headless, job-local TMPDIR/target; no full repo or sibling gate):

- `timeout 60s python3 -I scripts/test_pinned_sibling.py -v`: 11 passed, including missing/mismatched layout/cache and refusal to fall back to a live checkout even at the pin.
- `timeout 1000s python3 -I scripts/test-pin-readers.py`: 179 passed across all 13 reader binaries, none ignored/filtered, with an empty live sibling and pin `ecaaac060c44e317d1181345fa6b8e38c096c099`. The script uses Cargo `-j3 -- --test-threads=4`. Cold compilation took 4m10s; the oracle binary took 300.23s.
- `BENNETT_GATE_LAYOUT="$PWD/.tmp/gate/cache/bennett-isa-spec@ecaaac060c44" timeout 60s .tmp/rev-pin-circuits-r1/target/pin-readers/debug/deps/lowering-76f5d3c526d459f1 --test-threads=4`: 11 passed while the temporary consumer still existed.
- `git diff --check main...job/pin-circuits`: passed. Red-first assessed from `git show main:<reader>`: those readers access the deliberately empty live directory, so the isolation test would fail there; main was not checked out or executed.

P2+ nits: **`bennett-circuits-56n`** (P3, review/nit): README.md:18 needs pinned-cache/materialisation instructions; later Round 1 items 9–10 remain pending (gate.toml:10 has no quality ratchet; HANDOFF.md is still tracked and README.md:7 references it). Those later remediation items do not block this R0.2 job.

Coverage: read closely all 19 changed files, main-side reader paths, `d2fd693`, pin/gate conventions, and relevant published expectations; skimmed unchanged semantic test bodies. Checked every COMMON Anti-slop rule: dispatch/state, failure defaults, predicates/constants, function altitude/wrappers, process identifiers/litter, and test sensitivity/oracle independence; no blocking violation.

Provenance: implementer gpt-6.1-sol; reviewer gpt-6.1-sol, fresh session (D9). Beads filed in the main circuits checkout; no implementation fixes.
