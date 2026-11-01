# Circuit exporters

All three exporters consume `ir::Program`. The CLI checks stored object
identities and source obligations before lowering an action or specialising
an oracle. Classical arguments and budgets use the same syntax in every format:

```sh
bennett-circuits export-qasm object.bisa action
bennett-circuits export-qir object.bisa action --arg token:1,0,0,0
bennett-circuits export-quil object.bisa oracle --budget 4,0,0,0
```

An oracle requires an explicit budget. An action receives tokens through
`--arg`, in signature order. Embedded schedules retain the common §11 library
check; choosing another export format cannot bypass it. Draft certificate
input still requires the existing explicit draft opt-in.

QIR and Quil flatten the shared hierarchy into executable instructions. Their
versioned `bc:` comments preserve the complete source IR, including allocation
kinds, call hierarchy, own controls and attached source certificates. A separate
`bc-layout:` comment describes physical indices and the interface/workspace
partition. This is transport of source proof material, not a certificate for a
new target or schedule. Workspace and borrowed pools use ordinary qubits; a
standalone zero-initialised run has the same entry convention as `run-action`.
A caller can initialise the declared physical wires before invoking the body.
A zero-wire identity requires no sentinel in either format.

`qir::parse` and `quil::parse` recover the exact hierarchical IR and check the
canonical executable against it. Their separate `parse_flat` implementations
read actual gate calls/instructions and the layout, ignoring the IR comment.
These are validators for the emitted subsets, not general import APIs. They
reject unknown instructions, invalid indices, operand aliasing, malformed
angles and incomplete output. Removing an IR comment does not prevent
executable simulation; changing executable gates is visible to that simulation.

## QIR

The output is unrestricted QIR 1.0 using typed `%Qubit*` handles with consecutive
static integer indices, zero result handles and no measurement. It declares
64-bit pointers in its LLVM data layout. QIR profiles and QIS are distinct:
this exporter needs the QIS and runtime below and does not claim Base Profile
compatibility. See the [QIR specification](https://github.com/qir-alliance/qir-spec/blob/main/specification/README.md)
and [instruction-set contract](https://github.com/qir-alliance/qir-spec/blob/main/specification/Instruction_Set.md).

Single-qubit calls use `__quantum__qis__{x,h,s,t,rx,ry,rz}__body`, S/T adjoints,
and controlled/controlled-adjoint variants. Controls use `%Array*` values from
`array_create_1d`, `array_get_element_ptr_1d` and
`array_update_reference_count`; every array is released. Controlled rotations
use the QIR 1 tuple ABI: an array followed by a pointer to
`{ double, %Qubit* }`. These signatures are exercised in the QIR Alliance's
[controlled-signature tests](https://github.com/qir-alliance/qir-runner/blob/main/runner/tests/controlled_signatures.rs).

Negative controls use X conjugation. CZ appends its first target to a controlled
Z's controls; SWAP uses three controlled X calls. These identities introduce no
rotation arithmetic or global-phase approximation. Other gates retain their
original QIS operations. Angles are exact hexadecimal binary64 bit patterns,
including signed zero and subnormals. Array memory is classical; no extra
quantum ancillas are allocated by the exporter.

## Quil

The output uses the standard X/H/S/T/CZ/SWAP/RX/RY/RZ gates, `DAGGER` for S/T
adjoints, repeated `CONTROLLED` modifiers and X conjugation for negative
controls, followed by `HALT`. Rotation arguments use round-tripping decimal
binary64 literals. See the [Quil specification](https://quil-lang.github.io/).
No custom gate matrices, approximate decompositions or extra qubits are needed.

## Validation

`tests/exporters.rs` covers every gate, control polarity/count, adjoints, tiny
angles, zero-wire output, CLI actions/tokens/oracles, stale metadata and direct
executable mutation. LLVM 18 `llvm-as-18` and `opt-18 -passes=verify` validate the
all-gates QIR when installed; absence is reported explicitly. The job separately
assembled and verified the same module with LLVM 23 (`llvm-as-23`, `opt-23`).
The job also
checked for Quil/pyQuil, quilc, QVM, pyqir and qirrunner: none was installed.
LLVM validation establishes LLVM well-formedness, not quantum execution.

`tests/differential.rs` discovers all reference fixtures and adds supplemental
calls, workspace, memory and gate cases. It simulates the independently parsed
QIR, Quil and QASM on every interface basis tuple and compares exact states,
including phase and workspace, against the unexported circuit and placement
passes. Reference CLI display values are compared at absolute tolerance 2e-12;
that display comparison is evidence, not a numerical proof. The current corpus
contains 24 actions and 940 exhaustive basis tuples, including explicit
one-step token bindings for the re-imported oracle fixtures.


The test profile uses optimization level 1, with debug assertions and overflow
checks explicitly enabled. This keeps the existing multi-million-gate oracle
checks practical without reducing their domains or checks. The initial
unoptimised full run exceeded its 900-second bound; final results are recorded
in the job Resume block and Beads closure reasons.
