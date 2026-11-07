# Quality circuits review, round 2 — 2026-11-07

Verdict: **BLOCK**.

Reviewed fix range: `f391f301dcd037ef0ab7c0df08838a0df022a10c..68f64de8b268efa3fff93732c6400356829ffaf9`.
Reviewed head: **`68f64de8b268efa3fff93732c6400356829ffaf9`**.
Inspected `main..job/quality-circuits` history and the full-diff inventory;
review otherwise limited to this fix and the round-1 P1 reproduction.

Prior **P1-1**, `bennett-circuits-drw`: **fixed**.
Replayed the original 320-conditional-line mutation in memory against both
pre-fix and head scanners/inventories. Pre-fix omits `sha256` and reports no
regression. Head measures the original as `('sha256', 68, 9, 0)` and rejects
the mutation: over-300 bucket increases, maximum length 388 > 68, and file
complexity mass 330 > 10. The recorded baseline equals the complete head
inventory exactly; its increases account for three newly measured functions.

**P1-1 — const-expression operators are mistaken for generic delimiters**
(`bennett-circuits-n0p`), `scripts/quality_metrics.py:44-48`.
Every `<`/`>` is pushed/popped as an angle delimiter, including comparisons
and shifts in const expressions. Consequently the quality push check crashes
on valid Rust: changing only the existing SHA-256 return type from
`[u8; 32]` to equivalent `[u8; 1 << 5]` raises
`ValueError('unbalanced signature for function sha256')`.
The complete modified SHA module compiles successfully with Rust, and the
pre-fix gate entry returns 0 for the same in-memory edit. This is a reachable
gate regression introduced by the fix. Array comparisons and const-generic
shifts also reproduce it. Distinguish const-expression operators from generic
delimiters and cover those valid signatures.

Minimal reproduction run at head (exit 1):

```sh
timeout 30s python3 -I -c 'import runpy; q=runpy.run_path("scripts/quality_metrics.py"); print(list(q["functions"]("pub fn shifted() -> [u8; 1 << 5] { [0; 1 << 5] }")))'
```

Output: `ValueError: unbalanced signature for function shifted`.
The inventory/gate reproduction used `Path.read_text` patched in memory,
called `main()` with no CLI arguments, and compiled the entire modified
module via `timeout 20s rustc --edition=2021 --crate-name sha_signature_probe
--crate-type=lib --emit=metadata -o
.tmp/rev-quality-circuits-r2/tmp/sha_signature_probe.rmeta -`.
The enclosing Python probe ran under `timeout 30s`; no product files changed.

P2+ nits: none new; `bennett-circuits-7a2` records the two unchanged round-1
items from `bennett-circuits-cyh` (redundant nonempty assertions and the
grandfathered hook backup).

Validation: `timeout 30s python3 -I scripts/quality_metrics.py --self-test`
passes all seven tests; `timeout 30s python3 -I scripts/quality_metrics.py`
passes the baseline. Both miss the valid-operator case above.
`git diff --check f391f30..68f64de` passes. Commands ran headless with
reviewer-local TMPDIR/CARGO_TARGET_DIR. No full gate, Rust suite, coverage,
or sibling gate was run for this Python/documentation-only fix.

Coverage: read closely every fix line in the scanner, tests and baseline
changes, the scanner's inventory/gate consumers, the SHA function, and all ten
anti-slop rules; skimmed the full-batch diff inventory. No other anti-slop,
test weakening, owned-path or capability-narrowing issue found in this fix.
Main predates the metrics script; the red-side comparison used
`git show f391f30:scripts/quality_metrics.py` without switching branches.

Provenance: implementer gpt-6.1-sol; reviewer gpt-6.1-sol, fresh session (D9).
