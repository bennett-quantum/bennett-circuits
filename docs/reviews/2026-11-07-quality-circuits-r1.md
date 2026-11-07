# Quality circuits review, round 1 — 2026-11-07

Verdict: **BLOCK**.

Range: full `main..job/quality-circuits` (11 commits), diff `main...job/quality-circuits`;
merge base `ee20847f2406ec155d62b98432449da571e7ea48`.
Reviewed head: **`3b8627d8c7dd90a900eed2cb9f588e636702570a`**.
The review commit is outside that reviewed range.

**P1-1 — quality ratchet silently omits array-signature functions**
(`bennett-circuits-drw`), `scripts/quality_metrics.py:33-35`.
The first semicolon before a function's opening brace is classified as a trait
declaration terminator, including semicolons inside array types. Thus the
existing `src/sha256.rs:18`, `sha256(data: &[u8]) -> [u8; 32]`, contributes
no function length, complexity or wrapper measurement. Adding 320 conditional
lines leaves the complete inventory identical and `regressions() == []`;
the push check accepts growth it promises to reject (programme item 9).
Parse signature delimiters before deciding whether a semicolon terminates a
declaration, and add an inventory-level regression for this existing function.

Reproduction run from the worktree (in memory; no product files changed):

```sh
timeout 30s python3 -I -c 'import runpy; from pathlib import Path; q=runpy.run_path("scripts/quality_metrics.py"); s=Path("src/sha256.rs").read_text(); t=s.replace("    let mut h:", "    if data.is_empty() {}\n"*320+"    let mut h:", 1); a=list(q["functions"](s)); b=list(q["functions"](t)); print("functions:", [x[0] for x in a], "metrics unchanged:", a==b); assert a==b and "sha256" not in [x[0] for x in a]'
```

Output: `functions: ['hex', 'sha256_hex', 'known_vectors'] metrics unchanged: True`.
A separate `Path.read_text` monkeypatch/inventory run confirmed identical
whole-repository metrics and no reported regressions. The baseline check and
all four current self-tests pass, so those tests do not detect this failure.

P2+ nits (`bennett-circuits-cyh`): redundant nonempty-count assertions in
`tests/diagnostics.rs:17` duplicate the helper's existing invariant; the
pre-existing tracked `.beads/hooks/pre-push.backup` remains grandfathered
(already assigned to `bennett-circuits-8n7`, outside implementer ownership).

Validation: all commands ran headless, with reviewer-local TMPDIR and
CARGO_TARGET_DIR; sibling inputs resolved pin `ecaaac060c44e317d1181345fa6b8e38c096c099`.
`timeout 240s cargo test --offline -j3 --lib -- --test-threads=4`: 31 passed.
The equivalent integration command with filter `exporters::`: 8 passed.
For each of `proof_output::`, `static_checker::`, `differential::`,
`pinned_siblings::`, `diagnostics::`, `placement::`, `target::`,
`check_agreement::`, ran
`timeout 180s cargo test --offline -j3 --test integration <filter> -- --test-threads=4`:
135 passed, one promised test ignored. Total: **174 passed**.
The real LLVM-validator test with empty PATH fails explicitly (exit 101);
`git show main:tests/exporters.rs` confirms main silently returned on missing
LLVM. The `promised_6kc_` filter with `--ignored --test-threads=4` remains
red with §16.1/§5.3 (exit 101). Baseline/self-tests and `git diff --check`
pass. Full gates, coverage and the two large oracle tests were not rerun.

Coverage: read closely every code/test/tool/config change, including all
history handler extractions against main (normalized body comparisons leave
only expression-block formatting), placement, CLI and all ten anti-slop rules;
skimmed historical document deletions and numeric coverage evidence.
Items 9–11 checked: ratchet blocked above; HANDOFF retired and README points to
the workspace handoff. Slow test domains are retained in the gate. No new
capability narrowing found; symbolic-object acceptance remains explicitly
promised, consistent with the pinned literal bounds contract (§§4.8, 5.3).

Provenance: implementer gpt-6.1-sol; reviewer gpt-6.1-sol, fresh session (D9).
