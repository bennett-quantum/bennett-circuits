"""Independent syntax oracles for generated OpenQASM 3 and Quil.

Run with the exact parser versions named by the Rust caller. These check
external grammar acceptance; the ISA CLI differential checks semantics.
"""
from pathlib import Path
import hashlib
import json
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


def validate_direct(path):
    raw = path.read_bytes()
    ast = openqasm3.parse(raw.decode('utf-8'))
    count = sum(isinstance(s, openqasm3.ast.QuantumGate) for s in ast.statements)
    digest = hashlib.sha256(b'bennett-qasm-sha256-chain-v1').digest()
    metadata = end = None
    for line in raw.splitlines(keepends=True):
        if line.startswith(b'// bc-end: '):
            assert end is None
            end = json.loads(line[len(b'// bc-end: '):])
        else:
            assert end is None, 'data after completion'
            digest = hashlib.sha256(digest + len(line).to_bytes(8, 'big') + line).digest()
            if line.startswith(b'// bc-stream: '):
                assert metadata is None
                metadata = json.loads(line[len(b'// bc-stream: '):])
    assert metadata is not None and metadata['version'] == 'bc-oracle-stream-1'
    assert end is not None and end['scheme'] == 'sha256-chain-v1'
    assert end['digest'] == digest.hex()
    assert int(end['gates']) == int(metadata['counts']['total']) == count
    require_rejection(openqasm3.parse, 'OPENQASM 3.0; qubit q; x !!!;', QASM3ParsingError)
    print(f'external direct OpenQASM accepted: {count} gates; independent completion digest matches')


def main():
    if len(sys.argv) == 3 and sys.argv[1] == '--qasm-only':
        validate_direct(Path(sys.argv[2]))
        return
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
