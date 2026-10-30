State 2026-10-30: M1 complete; M2 IN PROGRESS. Sole implementer resumed after
infrastructure outage. No pushes, no external repository edits, reference CLI
only. Do not close bennett-circuits-pk3 or bennett-circuits-pk3.2 until the
remaining checker/certificate obligations below are actually implemented.

Latest resume: label-aware indirect matched cleanup implemented red-green;
`qjumpr` remains in its containing function by §4.2 even with lprocs in the
index space. Known labels survive moves/parmov/equal-arm selects. Check CLI
agreement is now 97/0; 30 static checker tests pass. Full gates to be rerun
after further work. Other M2 gaps below remain open.

Commits this session:
- c0221b5: non-permutation call-window events, wide domains, imported quantum
  signatures for CC calls.
- de0e01c: independent CC/R diagnostic execution, delta history, uncall and
  reversal, quantum handles/views/Born measurement; published A acceptance.
- e4eafed: oracle prefix reversal and independent direct row step-check,
  symbolic f64 adjoint proofs, separated borrow frames; B acceptance.
- 268fd6a: symbolic interface-call compositions and opaque imported contracts,
  canonical metadata/JCS/subclaim graph audits, classical allocation edges.

Latest conformance (pass/fail/unavailable): A 21/0/0; B 35/0/0; F 33/0/2.
F exits 2, not 0: F-frame-live-ranges and F-link-conflict are unavailable
in the published harness, which exposes no CLI probes for them. All sources,
cases and expected results were consumed read-only. Logs are in .tmp/m2-*.
Final gates: 84 tests passed / 0 failed / 0 ignored; formatting, strict
all-target Clippy and diff whitespace checks pass. See docs/PROGRESS.md.

Check agreement: 96 invocations agree / 0 diverge = 14 fixture/spec objects,
3 corrupted variants, 31 F check invocations, 42 conformance object files,
and 6 newly hashed/metadata witnesses. Categories repeat contents. Negative
conformance objects compare the deciding rule citation as well as verdict.
No known reference disagreement; there is no deciding-sentence divergence.
Action/oracle differential: 19 actions / 930 tuples and 59 occurrences /
488 tuples, all agree. Wide oracle interfaces remain explicitly sampled
four-pattern test evidence; no sampled proof/certificate claim is made.

Implemented checker improvements this session:
- Non-permutation call contracts close dependency windows even when their
  bodies contain only diagonal gates.
- Symbolic f64 actions use fresh uninterpreted tags and syntactic equality /
  negation, never numerical samples. Tags exclude literals in argument lists.
- Interface-only oracle/action contracts compose under qpred, ctl, adj, qif
  and literal qloop, checking effective controls and all bodies inductively.
  Opaque imported action calls are admitted only where no caller workspace
  obligation requires their implementation. Imported mut-region roles and
  CC handle signatures are checked from the published signature.
- Translation-lemma checking splits maximal permutation-only components;
  certificate fallback still streams all frame basis configurations and
  proves B restoration plus independence of other outputs from B.
- Wide full-D decimal bounds, duplicate domain names/cells, reversed bounds,
  cyclic/dangling subclaims, unsafe numeric metadata and UTF-16 JCS ordering.
- Controlled concrete action schedules fail explicitly under §6.4 until the
  complete inherited-context cleanup-key audit is implemented.

Still M2 (explicit errors, not scope removed or moved to M3):
- General symbolic integer/count/token actions with local workspace histories;
  multi-block action CFGs; arbitrary imported-call workspace provenance.
- Overlapping dynamic invocations at cyclic call points and interprocedural
  indirect-jump matched cleanup (§5.5).
- Borrow certificate expected-table/external-program/library/structural replay
  outside the bound exhaustive-current-program subset (§5.6/§12), imported
  qinplace inverse proofs, frame-descriptor schemas (§9.4), and full
  inherited-context cleanup schedule-key validation and recompute eligibility/
  schedule/certificate matching (§6.4).
- Remaining complete nested-schema/provenance audit. Do not infer exhaustive
  rule coverage from the finite published corpus.

Production diagnostics invoke no reference process. CC/R quantum amplitudes
are binary64 diagnostic values, not rigorous numerical proofs. CC entry types
and region classical arguments outside the implemented subset fail explicitly.
step-check directly evaluates arithmetic/cast/comparison/branch/address/load/
boundscheck/fault/retval rows independently of the circuit builder, exhaustive
at <=16 free input/memory bits (512 for tiny). Unsupported rows/wide direct
locations/large-domain sampling remain explicit staged errors. Oracle reverse
runs the compiled forward/commit/inverse circuit and verifies every workspace,
input and region bit before reporting the restored logical entry values.

Next milestone after actual M2 completion: M3 placement and scheduling.
