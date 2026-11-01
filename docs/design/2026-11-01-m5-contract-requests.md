# M5 contract requests

These requests remain in scope. They are not permission to weaken a test,
replace a full domain with a sample, invent generated-operation identities,
or call an unlinked proof complete.

The job can write only the circuits checkout. Attempts to search upstream
Beads with `bd search structural-only --limit 30`, `bd search eps_decade
--limit 20`, `bd search topology --limit 20` and
`bd --readonly search structural-only --limit 20` from `bennett-isa-spec`
all failed with:

```
workspacegate: open .../bennett-isa-spec/.beads.gate.lock: read-only file system
failed to open database ... b_isa_spec: openat LOCK: read-only file system
```

Therefore no claim is made that upstream requests are absent or filed. Local
owner **bennett-circuits-6ak** records the access failure and the required
upstream search/filing handoff. No upstream implementation was read or changed.

## Structural-only packets — local 9su

Exact request for ISA §12.3: “Specify the applicability and encoding of
`verification.oracle` and `verification.fault_rate` for a structural-only
action certificate with no exhaustive oracle relation. Make the fields
explicitly optional, or publish an explicit no-claim representation. Define
`bindings.reference_program_hash` for standalone actions as well as oracles.
Do not require an adjoint-table claim to stand in for an oracle/T3 relation.”

The published 0.9 schema still spells `oracle: i` and `fault_rate` on each
region, while §12.1 admits standalone structural adjoint/translation claims.
The current emitter cannot truthfully fill an oracle relation for a general
non-permutation action with only an adjoint proof. Nested qinplace proof-owner
composition also needs an unambiguous packet/owner binding rather than an
unsupported structural-only record. This is a contract question, not an
invitation to label numerical evidence as a proof.

## Target index — local v44

Exact request for ISA §8.1: “Define `eps_decade` as a total integer-valued
function of epsilon, including epsilon=0, powers of ten, boundary rounding
and the permitted domain. Specify evaluation using exact rational values,
independent of JSON decimal spelling.”

The 0.8 consumer continues to name v44 for the undefined index. The published
0.9 §8.1 K9 now supplies this definition: the exact least nonnegative j with
eps × 10^j ≥ 1 for 0 < eps ≤ 1, using the requested epsilon share. Its witness
covers 1, 0.1, 0.1001, 0.0999, 0.001 and 0.000999. Therefore a new request to
define the 0.9 index is unnecessary; the remaining task is deliberate target
version adoption, not an unresolved 0.9 formula. No private formula is inferred
from a reference implementation.

## Connectivity — local b4k

Exact request for ISA §8.3: “Publish the topology/connectivity/routing JSON
schema referenced by §8.2, including logical/physical vertex identities,
permitted edge/arity/control constraints, placement feasibility, routing
operations and how routing is charged and bound to a schedule. State the
semantics of a target with no topology.”

The 0.8 default remains all-to-all logical connectivity. The published 0.9
contract explicitly keeps routing deferred and rejects unavailable topology;
no claim of implementing that new-version target contract is made here.

## Reference draft-frame replay — local 4fa / 9xp.2

Exact request: “Identify a reference CLI/version that accepts the ratified
B5 `slots/scratch/history/borrow/ancillae` frame encoding and K13 positional
before/after tables under explicit draft acceptance, or specify the migration
of those draft packets to a published version. Report schema rejection
separately from certificate proof failure.”

The existing 0.8 source/0.9-draft certificate mode remains explicit. On the
current reference binary, check/hash rejects those packets either at the cert
version boundary or at the older frame schema (`missing field interface`).
The maintained tests preserve both exact rejection boundaries and separately
replay source basis rows. No reference certificate acceptance is claimed.

## Bound library — local yz8 / upstream b-isa-spec-8r4

The consumer is pinned to 0.8. Generated ordinals, supplied schedules,
cleanup/recompute/context composition, and numerical/channel claims require
the published library/version contract and a deliberate consumer adoption.
The presence of a newer ISA publication does not silently migrate this crate's
pinned source/object/target formats. Existing blockers remain open; this job
neither fabricates library bytes nor edits SPEC_VERSIONS.toml outside its brief.


## Heterogeneous interface-slot frames — local 4fa

Exact request for §9.4: “Specify how one signature slot spanning more than one
physical target class is represented by typed frame slots. Permit an ordered
class-fragment encoding with explicit slot identity and bit offsets, or define
a per-wire class vector for the slot; define width, alias and signature checks
for it. A single class per slot cannot describe a legal placement split across
classes when no individual class has sufficient capacity.”

The current B5 encoding has one slot record with a class and width, while
placement assigns classes to individual physical wires. The producer retains
a named refusal for an unrepresentable heterogeneous slot instead of claiming
one class for all of its wires. Homogeneous slots and scratch/history/borrow/
ancilla records are implemented.
