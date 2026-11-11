# bennett-circuits

## What this repo is

The bounded circuit specialiser: hierarchical circuit IR, placement, proof-carrying output, and exporters (OpenQASM 3, QIR, Quil).

It is one of several independent repositories in the Bennett reboot. It talks to
the others only through published specs.

## Where the spec is

Circuit-relevant sections: Bennett-ISA-Spec.md section 4.4 (gates), 8 (target 0.3), 12 (cert 0.3), 11 (sched 0.3); Bennett-NorthStar.md section 6.
The conformance suites for the versions in SPEC_VERSIONS.toml live in bennett-isa-spec.
Spec sources of truth: Bennett-ISA-Spec.md and Bennett-NorthStar.md in the Bennett.jl repo
until bennett-isa-spec is published.

## Rules

The one rulebook is bennett/docs/process/COMMON.md; it binds here. Gate: `scripts/pre-push`. Tasks: `bd` (`bd prime` lists its commands).
