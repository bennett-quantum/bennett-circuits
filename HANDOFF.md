State 2026-10-30: M1 complete; M2 closed by disposition at b4b28ac;
M3 target/default scheduling implementation landed at fe8f37f + 4025a21,
with final verification followup. M3 1ud stays open on publication dependencies.
Sole implementer; no pushes, Dolt remote sync or external repository edits.

Read CLAUDE.md, docs/PLAN.md, the latest docs/PROGRESS.md entry and
ISA 0.8 §§5/8/11/12. Only published spec contracts and reference CLI were used.

Implemented M3:
- Clean interval reuse after validation; interface/dirty-source/local pins.
- Target JCS/SHA-256, exact rational ASTs; own/effective control provenance.
- Concrete classes/peak capacity, native epsilon matching and first non-nesting
  schemes; clean/dirty scheme ancilla frames with disjoint reuse.
- Native QRAM costs exactly once, or explicit logical dispatch when qram=null.
- Default sequential leaf events; every declared count/time/occupancy/peak
  resource, class rates, dirty-rate replacement, zero-time capacity and QRAM
  predicate limits. Arbitrary nonterminating rational costs remain exact.
- placement::place_on_target and place_oracle_on_target. CLI check-target,
  place and place-qasm (oracle entries need --budget; --arg is positional).
- Placed QASM annotations retain source hierarchy/certificate, object/closure/
  target bindings, actual budget/arguments, map, class/time/resource data and
  proof obligations. Internal annotation fractions are not §12 decimals.
  bc-ir-2 records own controls; no new ISA §12 certificate is claimed.
- Exact action CLI state execution/reversal, one-shot launch and sub-byte
  action memory decoding. Source-embedded schedules never execute silently.

Publication dependencies / loud refusal paths:
- b-isa-spec-8r4: §11 bound library/ordinals and explicit scheduled realisations;
  consumers return E_SCHEDULE_INFEASIBLE with §11 and this id.
- bennett-circuits-4fa: §9.4/§12 frame and designated idle-pool JSON interchange.
- bennett-circuits-b4k (isa-0.9): §8.2 mentions topology, §8.3 has no topology,
  connectivity or routing encoding. Default remains all-to-all logical wires.
- bennett-circuits-v44 (isa-0.9): §8.1 admits eps_decade but never defines it.
- Exact default requests are supported; explicit nonzero schedules need the
  library and M4 numerical/channel claims. No private ref library was invented.

Five M2 decisions remain open unchanged: 7bu (symbolic workspace/CFG), z9r
(opaque workspace), 4pf (certificate/import replay), 4fa, yz8 (inherited/
recompute matching, also 8r4). Read docs/design/2026-10-30-m2-open-proofs.md.
M4 new certificates, M5 QIR/Quil, M6 scale/direct-machine generalisation stay
in scope. QIR/Quil commands explicitly say unimplemented, milestone 5.

Verification:
- 128 unique Rust tests passed, none failed/ignored: full 126-test run plus
  final companion 127-test run, with only the already executed large oracle
  case filtered in the companion. Fmt/strict all-target Clippy/diff clean.
- Check agreement 111/111; actions 19 / 930 tuples, zero divergences. Reuse,
  native, scheme and QRAM passes EACH agree with unplaced exact states on all
  930 tuples; every reference CLI comparison agrees. Fixtures alone 13/740.
- Oracles 59 occurrences / 488 tuples, placed/unplaced/reference all agree.
  Wide domains retain four-pattern evidence; sampled inputs are not proof.
- Conformance pass/fail/unavailable: A21/0/0 B35/0/0 C337/2/0 D7/0/6
  E0/0/16 F33/0/2 G10/0/14 H0/0/10. A/B exit0; C exit1; others exit2.
  Staged cases without probes remain counted unavailable, even where the new
  target library APIs independently exercise their scenarios.
- Two C diagnostic divergences, both tracked by bennett-circuits-fgx:
  C-step-check-sumdb-fixed-db needs fixed-region/exhaustive CLI options;
  C-step-check-wide-fault needs >128-bit direct literals. Both fail explicitly
  as unimplemented milestone 6. Deciding rules: §14.1 “Executes an oracle's
  reference body per basis configuration”; §5.2 “fr final = initial ⊕
  code(F_B(x))”. No expected result or semantic domain was narrowed.

Logs: .tmp/m3-final-tests.log (126/full, including 59/488),
.tmp/m3-final-other-tests.log (127/final companion), .tmp/m3-final-{A..H}.log,
.tmp/m3-final-clippy.log. Largest oracle test materialises >2M gates; allow
several minutes and roughly 5GB RAM. No remote automation was introduced.
