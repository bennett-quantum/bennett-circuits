# Quality evidence

## Contract audit

All source contracts were read through `scripts/pinned_sibling.py` at
`ecaaac060c44e317d1181345fa6b8e38c096c099`. Supported source/target versions
remain 0.8; bundled 0.9 text supplies an explicit draft proof mode, not implicit
version migration. The open contract and adoption tasks have clause-specific
notes in Beads (frame §9.4; certificate §§12.1–12.3; target §8; library §11.1).
The symbolic-loop experiment is not adopted: §4.8 lists wires, bundles,
regions and literals as Q instruction operands, §5.3 fixes text-order bounds,
and the pinned reference rejects `qloop n` with §16.1 (canonical unsigned INT).
The runtime extension binds integer counts before lowering; whole-object
acceptance needs an explicit parameter/bound contract. A named ignored promised
test retains the requested capability. Evidence: `symbolic-red.log` and
`symbolic-green.log` in the ignored job directory; the latter exposes two
reference-contract regressions from the experimental extension.

Further measurements and the quality baseline are recorded below as completed.
