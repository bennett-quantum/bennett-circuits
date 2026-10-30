M3 implementation checkpoint: §8 target loader/expressions, class/capacity,
native/scheme placement, sequential event accounting and annotated QASM now
implemented; full acceptance audit pending. Read latest docs/PROGRESS.md.
D green 7/0/5; new maintained D/G CLI probes green; target fixture differential
740/740, no divergences. Topology/routing gap b4k filed isa-0.9.

State 2026-10-30: M1 complete; M2 disposition closed; M3 first slice landed.
Sole implementer. No pushes, Dolt remote sync or edits to another repository.
Reference execution uses only its CLI; no code copied across the contract.

M2 commits: 3fb6893 (disposition and cyclic counts), 7e6baae (commuting
oracle deliveries), b4b28ac (named concrete CFG/imported refusal paths).
pk3/pk3.2 closed at explicit user direction after the no-fourth-deferral
inventory. This does NOT assert a complete general checker. Durable owners:
- (a) overlapping cyclic invocations implemented with minimum multiplicities;
  10 materialised reference check comparisons agree, including invalid cases.
- (b) symbolic workspace/CFG: bennett-circuits-7bu (design).
- (b) opaque workspace effects: bennett-circuits-z9r (design).
- (b) broader certificate/imported inverse replay: bennett-circuits-4pf (isa-0.9).
- (b) frame schema: bennett-circuits-4fa (isa-0.9).
- (b) inherited/recompute matching: bennett-circuits-yz8 (isa-0.9), plus
  unpublished expansion library **b-isa-spec-8r4**.

Read docs/design/2026-10-30-m2-open-proofs.md for exact spec sentences,
two readings/design choices and their costs. Every unsupported proof retains
a named unimplemented error. Symbolic and opaque-call representations are
implementation decisions, not impossibility claims or invented ISA gaps.
Frame/table encodings need publication. NorthStar scope remains required.

M3: `placement::place_action(&object, action, &args)` validates first and
returns a placed flat circuit, logical/physical map and half-open gate live
intervals. It reuses clean workspace only across disjoint intervals of the
same requested class, counting controls as uses. Interfaces, dirty borrowed
wires and local regions stay dedicated; unused clean wires need no slot.
Sequential scratch reduces 3 to 2 total wires; controlled call/adjoint scratch
reduces 4 to 3. Exact phase and interference tests pass, including arbitrary
dirty borrowed inputs. Hierarchical IR/QASM remain the source forms.
Target assignment (`any` is unresolved), schemes/epsilon, capacity, scheduled
resource vectors and exporter integration remain M3 `bennett-circuits-1ud`.
First-slice `bennett-circuits-pk3.4` is closed under M3. Supplied schedules
fail explicitly naming **b-isa-spec-8r4**; no fake ordinals/library introduced.

Verified: **107 tests passed / 0 failed / 0 ignored**; fmt, strict all-target
Clippy and diff checks pass. Check agreement **111/111**, no divergences.
Actions **19 / 930 tuples**, including placed exact states, all agree with
the CLI reference. Fixture-only placement separately covers **13 / 740**.
Oracles **59 occurrences / 488 tuples**, all agree. Wider domains retain
four-pattern sampling; comparisons are evidence, not certificates.
Conformance pass/fail/unavailable: **A21/0/0 B35/0/0 F33/0/2**; A/B exit0,
F exit2 for unchanged missing frame-live-range and link-conflict probes.
No unresolved reference disagreement or deciding-spec divergence.
Logs: .tmp/m3-final-tests.log and .tmp/m3-final-{A,B,F}.log.
