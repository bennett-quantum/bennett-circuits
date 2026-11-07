"""Independent syntax oracles for generated OpenQASM 3 and Quil.

Run with the exact parser versions named by the Rust caller. These check
external grammar acceptance; the ISA CLI differential checks semantics.
"""
from pathlib import Path
import sys

import openqasm3
from quil.program import Program, ProgramError
from openqasm3.parser import QASM3ParsingError


def require_rejection(parser, text, expected_error):
    try:
        parser(text)
    except expected_error:
        return  # Deliberately malformed input must raise from the oracle.
    raise AssertionError('external parser accepted the malformed control input')


def main():
    qasm, quil = map(Path, sys.argv[1:])
    ast = openqasm3.parse(qasm.read_text())
    assert ast.statements, 'OpenQASM parser returned no statements'
    program = Program.parse(quil.read_text())
    assert program.body_instructions, 'Quil parser returned no instructions'
    require_rejection(openqasm3.parse, 'OPENQASM 3.0; qubit q; x !!!;', QASM3ParsingError)
    require_rejection(Program.parse, 'X( 0', ProgramError)
    print('external OpenQASM/Quil syntax accepted; malformed controls rejected')


if __name__ == '__main__':
    main()
