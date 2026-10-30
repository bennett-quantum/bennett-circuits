# M2 open proof dispositions — 2026-10-30

Decision inventory taken during the first hour of this pass, against ISA 0.8.
The user explicitly authorises closing pk3/pk3.2 after this inventory, then
starting M3. Closure records disposition, **not completion of the full static
checker**. These capabilities remain required by NorthStar §5.1. No sampling,
finite corpus agreement, placeholder library or guessed metadata schema
substitutes for a universal proof.

| Open item | Disposition | Durable owner |
|---|---|---|
| General symbolic workspace / action CFG proofs | (b) proof representation / action CFG contract decision | bennett-circuits-7bu (`design`) |
| Overlapping cyclic invocations | (a) implemented red-green, §5.5 minimum outstanding multiplicities | pk3.2 |
| Opaque workspace effects | (b) unlinked proof-summary decision | bennett-circuits-z9r (`design`) |
| Broader certificate replay, including imported inverse proofs | (b) tuple encoding / resolver decision | bennett-circuits-4pf (`isa-0.9`) |
| Frame schemas | (b) published JSON encoding decision | bennett-circuits-4fa (`isa-0.9`) |
| Full §6.4 inherited-context / recompute matching | (b) operation-identity publication decision | bennett-circuits-yz8 (`isa-0.9`), upstream b-isa-spec-8r4 |

## General symbolic workspace and action CFGs

§6.2: “Each cleanup is bound to its exact computing sequence and requires the
versions that sequence read to be current.” §5.3's action body lists gates,
calls and structured `qif`/`qloop`; §16.2 permits multiple `block`s generically.
Neither defines an action branch terminator or execution/join rule for a
multi-block action. §5.3 bounds are text-order literal counts; serialized
parameter counts remain rejected with the reference's §16.1 citation.

There are two separable decisions. For CFGs, reading one treats all action
blocks as text-order unitary fragments (labels are descriptive); reading two
treats them as executable CFG nodes needing explicit coherent branch, join,
padding and adjoint semantics. The first costs a canonical flattening rule
and tests of cross-block lifetimes; the second costs an ISA grammar/semantics
extension. Inventing a jump semantics from lifted or CC rows is not justified.
Request: specify one interpretation for action blocks in the accepted ISA.

For symbolic workspace across calls there is **no missing semantic permission**
to implement §6, and no claim that general proofs are impossible. The design
choice is what “exact computing sequence” means in our proof IR: retain
versioned symbolic instruction histories (including argument/token atoms and
call/context keys), or retain canonical symbolic transfer expressions and
proof obligations discharged at instantiation. Histories require a new
versioned IR shared with cleanup checking and universal symbolic call summaries;
instantiation obligations require an explicit API carrying *undischarged*
obligations and a universal coverage rule for `check`/`hash`. Checking one
chosen token is not such a rule. The current gate IR stores concrete budgets
and f64 angles, so silently lowering a representative token cannot prove the
published rule. The requested decision is a proof-IR contract, not a relaxation
of §6. `7bu` owns this implementation design gap and the multi-block question;
existing independence and symbolic-angle proofs remain available.

## Opaque workspace effects

§6.1 gives the call transfer sets; §6.2 binds release of `other` to the adjoint
of its producer. §9.3 says “Linking = resolution, validation, certificate
composition (§12.3), rebinding to closure hashes.” An unpinned import has a
signature but no implementation identity until linking. There is no published
producer-side symbolic proof-summary representation.

Reading one proves opaque calls extensionally from signatures, using a stable
uninterpreted function identity for each output and exact forward/adjoint call
histories. That costs canonical symbol/closure, argument, token, control and
cell-version keys, including permutation and non-permutation transfers.
Reading two records pending body/certificate obligations during unlinked
checking and discharges them only after §9.3 resolution. That costs a pending
obligation API and linked certificate resolver; an unlinked hash must not be
misreported as a completed cleanup proof. Neither reading permits permanently
refusing imported code or guessing its effects. The spec's semantic transfers
are clear; the unresolved part is our proof-summary/link-stage contract.
`z9r` asks for this decision explicitly. The refusal names both `z9r` and
`7bu` rather than claiming signature validation proves workspace restoration.

## Certificate replay

§12.1 gives `expected: { "table": [...] }` and requires comparison of full
tuples; §12.2 gives input-domain enumeration order. It does **not** give the
JSON encoding of a table row, output tuple, region-cell ordering in that row,
or a frame-local table's binding to source frame endpoints.

Reading one is one ordered *output* tuple per enumerated domain element;
reading two is keyed `(input tuple, output tuple)` rows with explicit names.
The first costs a normative output slot order/width/borrow encoding and exact
length checks; the second costs uniqueness, full coverage and coordinate
validation. An empty or truncated table must never bypass frame factorisation.
A reference implementation's private encoding is not the contract (rule 5).

For `expected.program` and `inverse_pair_library`, a hash names a closure but
this crate has no specified external object/certificate resolver. Choose a
linked-object closure registry (cost: certificate DAG, binding/rebinding,
cycle and result checks) or an explicit replay-input bundle (cost: a new
checker API and authenticated closure-to-object mapping). Structural
translation/adjoint witnesses additionally need agreed source-point encoding
and call/loop mapping. Imported qinplace inverse proofs must use the same
resolver and quantify the actual budget; accepting a signature is not proof.
`4pf` requests those encodings and resolution inputs. Existing bound
current-program exhaustive full-D factorisation remains implemented. Library
paths additionally name **b-isa-spec-8r4**; no library bytes are fabricated.

## Frame schema

§9.4 enumerates interface slots, scratch by class/cleanliness as expressions
of `c`, retained history, borrow requirement and scheme ancillae. §12.3 says
only `frame (§9.4)`; §9.1 forbids unknown fields. There is no normative 0.8 JSON
field vocabulary, required/optional list, shared-frame live-range encoding or
idle-pool reference encoding.

Reading one uses typed records (`slots`, scratch/history/borrow/ancillae
categories); reading two uses per-class expression maps plus role/cleanliness
annotations. The first costs a larger recursive schema and explicit slot
identities; the second costs disambiguating categories, zero versus absence,
and alias/live-range rules. Both must restrict expressions to `c` and enforce
disjoint shared lifetimes. `4fa` asks for the exact JSON schema to publish;
this pass does not reverse engineer one from reference source or adopt the
unaccepted 0.9 draft. Any supplied `frame` retains an explicit named error.

## Inherited context and recompute

§6.4 requires keys `(region hash, variant, inherited control count)` and
recompute into a **fresh clean bundle** using the **same sequence**. §11 says
generated ordinals, scratch identities and removable operations come from the
bound expansion library. The `ref` document remains unpublished under
**b-isa-spec-8r4**. High-level source points cannot establish which generated
cleanup/allocation a wildcard selector actually covers.

Reading one publishes the library with stable generated cleanup and fresh
recompute identities, then resolves invocation paths and all inherited keys
against those operations (cost: library consumer and context enumeration;
no contract change). Reading two adds an independent high-level cleanup-point
identity contract to sched/cert (cost: versioned selector/schema change and
migration, plus proof that expansion realises the selected cleanup). `yz8`
requests the choice/publication; publication is the compatible route. Schema
checks alone are not semantic matching. Recompute/inherited-context errors
name both `yz8` and **b-isa-spec-8r4**. M3 schedule consumption that requires
these ordinals must fail explicitly for the same upstream bead.

## Implemented cyclic proof

§5.5 permits outcomes retained until prefix reversal. Identical call operands
can therefore have multiple outstanding dynamic invocations. The checker now
tracks guaranteed outstanding multiplicity per operand signature, separately
for each propagated label state, and takes componentwise minima at CFG joins.
Call pushes and uncall pops are monotone, as is invalidation by writes; a
positive minimum proves a matching invocation on *every* incoming path.
Increasing loop counts do not force unbounded enumeration; a previously seen
smaller count subsumes them. Ordinary writes invalidate read/delivery versions;
XOR deliveries preserve stack multiplicity and commute for different input
signatures when those inputs stay current. Ten materialised, correctly
hashed witnesses agree with the reference's `check`, including negative
version and over-cleanup cases. No reference code was copied or linked.
