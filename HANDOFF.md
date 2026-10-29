State (2026-10-29): M1 complete; M2 still IN PROGRESS. Do not close pk3 or
pk3.2. Latest resumed work is recorded at the end of docs/PROGRESS.md.

New in this session: direct-CFG matched cleanup (balanced loops/path joins),
qreg handle worklists and block-argument/view renaming, unreachable schema
checks, imported lproc/classical call contracts, recursive metadata validation,
selector/while-dependency immutability, and fixes for quantum cleanup falsely
accepting H–S–S–H or H–CX–H. All three new negative witnesses agree with ref.
Borrow validation now supports bound exhaustive certificates whose expected
program is the actual current closure, verifies full D, and exhaustively
proves factorisation of every permutation frame. Translation lemma retained.
Expected-table/external-program/library/structural certificate replay remains
an explicit M2 error; certificate frame descriptors remain an explicit error.

Diagnostic run-oracle now uses basis propagation and halted-prefix expansion,
so published depth/stack-limit cases complete without multi-GB allocations.
Exported oracle circuits retain the fixed-budget/full-D construction.

Comparisons: 93 check invocations agree, 0 diverge (14 fixture/spec + 3 corrupt
+ 31 F + 42 conformance object files + 3 new witnesses; repeated contents).
Oracle/action harness checkpoints: 59 occurrences / 488 tuples and 19 actions /
930 tuples, all agree. Final gate and conformance reruns: see PROGRESS.md.
Published suite B: 31 pass / 4 fail / 0 unavailable. Failures are three reverse
CLI probes and the independent step-check probe. Suite A: 0 pass / 21 fail
(no run-cc/reverse/history CLI). F checkpoint: 33 pass / 0 fail / 2 unavailable.

Still M2: full symbolic parameterized actions and opaque arbitrary-call
provenance/cleanup; overlapping cyclic matched invocations and interprocedural
indirect jumps; remaining imported-region and qreg call/return contracts;
complete certificate/library replay and nested schema/canonical audits;
full §6.4 inherited-context keys; the A/B execution CLI acceptance probes.
Existing explicit M2 errors are unfinished work, not scope reductions.
Next milestone after actual M2 acceptance: M3 placement and scheduling.
No pushes; no external repository edits; reference implementation CLI only.
