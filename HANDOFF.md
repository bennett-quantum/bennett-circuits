State 2026-10-30: M1 complete; M2 IN PROGRESS; M3 unstarted.
Sole implementer. No pushes, Dolt remote sync or edits to another repository.
The reference is used through its CLI only. Do not close bennett-circuits-pk3
or bennett-circuits-pk3.2: the remaining obligations below are not completed.

Latest commits: 521e42e (label-aware indirect cleanup), 0bfd28d (universal
parameter-independent workspace sections), followed by the schema/library
refusal and specialisation-bound audit commit. See docs/PROGRESS.md for the
complete history, deciding sentences, counts and limitations.

Verified gates: 94 tests passed / 0 failed / 0 ignored. Formatting, strict
all-target Clippy and diff whitespace checks pass. Logs under .tmp:
m2-resume-final-tests.log and m2-resume-final-{A,B,F}.log.

Conformance (pass/fail/unavailable): A 21/0/0, B 35/0/0, F 33/0/2.
A/B exit 0; F exits 2. F-frame-live-ranges and F-link-conflict are unavailable
because the published harness has no execution probes. They are not passes.
The conformance crate, cases and expected results were consumed read-only.

Check agreement: 101 invocations agree / 0 diverge, including all fixture/spec
and conformance objects and negative rule citations. Counts include repeated
contents: 14 fixture/spec objects + 3 corruptions + 31 F checks + 42 conformance
objects + 11 additional witnesses. No known reference error or unresolved
reference disagreement; no deciding-spec divergence to report.

Differential: 19 actions / 930 tuples and 59 oracle occurrences / 488 tuples,
all agree, zero divergences. Wide oracle interfaces retain the documented
four-pattern sampling limits. These are test evidence, not certificates.

Implemented in this resume:
- Path-sensitive label provenance through label/mov/parmov/equal-arm select
  resolves indirect cleanup destinations. qjumpr stays in its containing
  function by §4.2 even when lprocs occur in the oracle index space.
- Workspace histories with syntactically unused integer/token parameters
  (including mixed f64 parameters) are universally proved, without choosing
  integer/token samples. Independently clean workspace sections compose with
  symbolic interface-only calls. Imported interface contracts need no body.
  Workspace spanning a symbolic call remains an explicit §6 M2 proof gap.
- Certificate cleanup/error context keys, composition multiplicity keys and
  recursive invocation-path schemas are checked. This does not claim full
  inherited-context or generated-operation matching.
- Raw lowering compares resolved symbolic loop counts to declared identity
  bounds before unrolling. Serialized nonliteral qloop counts retain an
  explicit §16.1/§5.3 M2 refusal, agreeing with the reference's citation.
- Library-dependent recompute/inherited-context matching and inverse-pair
  replay errors now name b-isa-spec-8r4. The reference library is unpublished
  until 0.9; no fake hash, ordinal set, library or proof has been introduced.

Still M2, with explicit errors (not scope removed or moved to M3):
- General symbolic workspace histories across calls/control structure,
  multi-block actions and arbitrary opaque imported workspace provenance.
- Overlapping dynamic invocations at cyclic call points (§5.5).
- Expected-table/external-program/library/structural certificate replay
  outside the bound exhaustive-current-program subset (§5.6/§12), imported
  qinplace inverse proofs, frame descriptor schemas (§9.4), inherited-context
  cleanup schedule coverage and recompute eligibility/matching (§6.4).
- The complete remaining nested-schema/provenance audit. Passing finite suites
  does not discharge these obligations. pk3 and pk3.2 remain in progress.

Earlier concrete §6.1/§6.2 provenance, cell-version/window, non-permutation
call events, quantum-use histories, §6.3 T5, effective-control disjointness,
exact adjoints and measurement-cleanup rejection remain implemented. Borrow
translation frames and bound exhaustive-current-program full-D factorisation
are supported. See PROGRESS for the precise existing inventory.

Production diagnostics invoke no reference process. CC/R quantum amplitudes
are binary64 diagnostic values, not rigorous numerical proofs. Unsupported
CC entry/region arguments and direct-machine step-check rows/widths remain
explicit staged errors. M3 placement/scheduling follows actual M2 completion.
