State (2026-10-29): M1 complete; M2 is IN PROGRESS. Do not close pk3 or
claim the full static checker. ISA/object/launch/sched/cert/coverage/target 0.8.
Plan: docs/PLAN.md. Exact progress, domain policies and gaps: docs/PROGRESS.md.

Implemented: literal bounded oracle gate construction, all §4.2 rows, canonical
SCC/label indices, lproc stack/capture/delivery, allocator/memory checks, nested
oracle/permutation calls, exact prefix cleanup, and budget/argument-specific
hierarchical action calls. qinplace proves both compositions and fault freedom
exhaustively under its actual token; proof metadata binds object/closure hashes.
ISA §12 serialization and verified incoming library certificates remain staged.

CLI: check/hash, budget/argument-aware export-qasm; compiled basis run-oracle
(with exact active-row count) and permutation-only basis run-action/adjoint.
Non-permutation diagnostic CLI simulation fails explicitly; the independent
exact test simulator still covers all exported gate families.

Verified oracle corpus: 59 occurrences in 12 fixtures + 2 spec objects;
488 full-interface basis tuples, all agree, zero divergences. <=10-bit interfaces
exhaustive; larger interfaces use four explicit patterns, not full-D proof.
M1 action harness remains 19 actions / 930 comparisons, all agree.
Checker: 14 corpus objects + 3 corrupt variants + 31 F check cases agree (48
verdict comparisons). Published Suite F harness: 33 pass / 0 fail / 2 unavailable,
exit 2. scripts/run-conformance compiles unchanged external harness sources with
local output/run paths; never writes the spec repository.

M2 remains: general symbolic classical-action provenance/cleanup, CFG-aware
§5.5 matched cleanup, qreg handle liveness at CFG joins, incoming certificate-
backed borrow proofs and imported-schema cases; complete nested metadata,
selector immutability and arbitrary-call provenance audits. Existing explicit
M2 errors are intentional incomplete-work diagnostics, not scope reductions.
Local gates: 49 tests passed; fmt, strict Clippy and diff checks clean.
Beads: pk3.1/pk3.3 closed; pk3.2 and pk3 remain in progress.
Next milestone after M2 acceptance: M3 placement and scheduling.
No pushes; no external repository edits or interpreter source reads.
