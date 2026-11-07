#!/usr/bin/env python3
"""Ratchet Rust structure and tracked-file hygiene against docs/quality.md.

Complexity is a reproducible lexical proxy: control-flow tokens, weighted by
brace nesting inside each function. It is not a semantic complexity analyser.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
MARKER = '<!-- quality-baseline -->'
PROCESS = re.compile(r'\bbennett-circuits-[a-z0-9]{3}(?:\.[0-9]+)?\b|\bb-isa-spec-[a-z0-9]{3}\b|\b(?:milestone|shift|round|job)[ -][0-9]+\b|\bM[1-9]\b|\bquality-[a-z]+\b|\b20[0-9]{2}-[0-9]{2}-[0-9]{2}\b', re.I)
LITTER = re.compile(r'(^|/)(__pycache__|PROGRESS[^/]*|LAST.RUN[^/]*|LOCK)(/|$)|\.(pyc|log|darc|backup|lock)$')
TOKEN = re.compile(r'\br(?P<hash>\#*)".*?"(?P=hash)|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'|//[^\n]*|/\*.*?\*/', re.S)
FUNCTION = re.compile(r'\bfn\s+([A-Za-z_][A-Za-z_0-9]*)\s*(?=[<(])')
SIGNATURE = re.compile(r'->|[()[\]<>{};]')  # A return arrow does not close a generic.
CLOSING = {')': '(', ']': '[', '>': '<', '}': '{'}
BRANCH = re.compile(r'\b(if|else|match|for|while|loop)\b|&&|\|\||\{|\}')


def mask(text):
    # Preserve offsets and line numbers while excluding comments/literals.
    return TOKEN.sub(lambda m: ''.join('\n' if c == '\n' else ' ' for c in m[0]), text)


def functions(text):
    clean = mask(text)
    for match in FUNCTION.finditer(clean):
        delimiters = []
        for token in SIGNATURE.finditer(clean, match.end()):
            word = token[0]
            if word == '{' and not delimiters:
                start = token.start()
                break
            if word == ';' and not delimiters:
                start = None
                break
            if word in '([<{':
                delimiters.append(word)
            elif word in CLOSING:
                if not delimiters or delimiters.pop() != CLOSING[word]:
                    raise ValueError(f'unbalanced signature for function {match[1]}')
        else:
            raise ValueError(f'unterminated signature for function {match[1]}')
        if start is None:
            continue  # Trait declaration, not a body.
        depth, end = 1, start + 1
        while depth and end < len(clean):
            depth += (clean[end] == '{') - (clean[end] == '}')
            end += 1
        if depth:
            raise ValueError(f'unclosed function {match[1]}')
        body = clean[start + 1:end - 1]
        nesting, mass = 0, 0
        for token in BRANCH.finditer(body):
            word = token[0]
            if word == '{':
                nesting += 1
            elif word == '}':
                nesting -= 1
            else:
                mass += 1 + nesting
        length = clean.count('\n', match.start(), end) + 1
        # Single-expression forwarding functions, allowing return/?/semicolon.
        wrapper = int(bool(re.fullmatch(r'\s*(?:return\s+)?[\w:.]+\([^{};]*\)\??;?\s*', body)))
        yield match[1], length, mass, wrapper


def inventory(root):
    tracked = subprocess.check_output(['git', '-C', str(root), 'ls-files', '-z'], text=True).split('\0')
    files = sorted(set(tracked) | {str(p.relative_to(root)) for p in (root / 'src').rglob('*.rs')})
    distribution = dict.fromkeys(['1-30', '31-100', '101-300', 'over-300'], 0)
    mass, longest, wrappers = {}, {}, 0
    for name in files:
        path = root / name
        if not name.startswith('src/') or path.suffix != '.rs' or not path.is_file():
            continue
        rows = list(functions(path.read_text()))
        mass[name] = sum(row[2] for row in rows)
        longest[name] = max((row[1] for row in rows), default=0)
        wrappers += sum(row[3] for row in rows)
        for _, length, _, _ in rows:
            bucket = '1-30' if length <= 30 else '31-100' if length <= 100 else '101-300' if length <= 300 else 'over-300'
            distribution[bucket] += 1
    identifiers = sum(len(PROCESS.findall((root / name).read_text())) for name in files
                      if name.startswith('src/') and (root / name).is_file())
    return dict(length_distribution=distribution, longest_per_file=longest,
                complexity_mass_per_file=mass, trivial_wrappers=wrappers,
                process_identifiers=identifiers,
                tracked_litter=sorted(name for name in files if LITTER.search(name) and (root / name).exists()))


def regressions(current, baseline):
    errors = []
    # Short functions can be added. Only the long tail ratchets downward;
    # wrappers and complexity prevent cosmetic splitting from hiding growth.
    for bucket in ['31-100', '101-300', 'over-300']:
        if current['length_distribution'][bucket] > baseline['length_distribution'][bucket]:
            errors.append(f'function length bucket {bucket} increased')
    for metric in ['longest_per_file', 'complexity_mass_per_file']:
        for name, value in current[metric].items():
            if value > baseline[metric].get(name, 0):
                errors.append(f'{metric}: {name}: {value} > {baseline[metric].get(name, 0)}')
    for metric in ['trivial_wrappers', 'process_identifiers']:
        if current[metric] > baseline[metric]:
            errors.append(f'{metric}: {current[metric]} > {baseline[metric]}')
    for name in set(current['tracked_litter']) - set(baseline['tracked_litter']):
        errors.append(f'tracked litter added: {name}')
    return errors


class MetricsTests(unittest.TestCase):
    def test_literals_comments_and_nested_control(self):
        rows = list(functions('fn run() {\n if a { while b { f("}"); } }\n // if x {\n}\n'))
        self.assertEqual(rows, [('run', 4, 3, 0)])

    def test_wrapper_and_trait_declaration(self):
        self.assertEqual(list(functions('fn abstracted(); fn forward(x: u8) { other(x) }')),
                         [('forward', 1, 0, 1)])

    def test_signature_delimiters_distinguish_bodies_from_declarations(self):
        signatures = [
            'fn typed(input: [u8; 4]) -> ([u8; 32], [u16; 2])',
            'fn typed() -> Option<[u8; 32]>',
            'fn typed<T: Into<[u8; 32]>>(input: T) -> [u8; 32]',
            'fn typed<T>(input: T) -> [u8; 32] where T: Into<[u8; 32]>',
            'fn typed(input: fn() -> [u8; 32]) -> [u8; 32]',
            'fn typed<T: Fn() -> [u8; 32]>(input: T) -> [u8; 32]',
            'fn typed() -> [u8; { let n = 4; n }]',
        ]
        for signature in signatures:
            with self.subTest(signature=signature, kind='body'):
                self.assertEqual(list(functions(signature + ' {\n if ready { work(); }\n}')),
                                 [('typed', 3, 1, 0)])
            with self.subTest(signature=signature, kind='declaration'):
                trait = 'trait Hash { ' + signature + '; fn forward() { other() } }'
                self.assertEqual(list(functions(trait)), [('forward', 1, 0, 1)])

    def test_malformed_signatures_fail_loudly(self):
        for source in ['fn broken(x: [u8; 4) {}', 'fn broken(x: [u8; 4]']:
            with self.subTest(source=source):
                with self.assertRaisesRegex(ValueError, 'signature.*broken'):
                    list(functions(source))

    def test_sha256_growth_is_rejected_by_inventory(self):
        path = ROOT / 'src/sha256.rs'
        read_text = Path.read_text
        source = read_text(path)
        insertion = '    let mut h:'
        self.assertIn(insertion, source)
        mutated = source.replace(insertion, '    if data.is_empty() {}\n' * 320 + insertion, 1)
        baseline = inventory(ROOT)
        with patch.object(Path, 'read_text', lambda p: mutated if p == path else read_text(p)):
            current = inventory(ROOT)
        errors = regressions(current, baseline)
        self.assertTrue(errors, 'growth in the existing sha256 function must fail the ratchet')
        self.assertIn('function length bucket over-300 increased', errors)
        self.assertTrue(any('complexity_mass_per_file: src/sha256.rs:' in e for e in errors))

    def test_process_identifiers_exclude_published_format_names(self):
        self.assertFalse(PROCESS.search("bennett-circuits-placement-1"))
        self.assertTrue(PROCESS.search("bennett-circuits-abc"))
        self.assertTrue(PROCESS.search("milestone 3"))
        self.assertTrue(PROCESS.search("quality-circuits"))

    def test_each_metric_rejects_growth(self):
        base = dict(length_distribution={'31-100': 0, '101-300': 0, 'over-300': 0},
                    longest_per_file={'src/a.rs': 4}, complexity_mass_per_file={'src/a.rs': 3},
                    trivial_wrappers=0, process_identifiers=0, tracked_litter=[])
        self.assertEqual(regressions(base, base), [])
        changes = [('length_distribution', {'31-100': 0, '101-300': 0, 'over-300': 1}),
                   ('longest_per_file', {'src/a.rs': 5}),
                   ('complexity_mass_per_file', {'src/a.rs': 4}),
                   ('trivial_wrappers', 1), ('process_identifiers', 1),
                   ('tracked_litter', ['src/file.log'])]
        for key, value in changes:
            with self.subTest(metric=key):
                self.assertTrue(regressions({**base, key: value}, base))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--measure', action='store_true')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(MetricsTests)
        return 0 if unittest.TextTestRunner().run(suite).wasSuccessful() else 1
    current = inventory(ROOT)
    if args.measure:
        print(json.dumps(current, indent=2, sort_keys=True))
        return 0
    text = (ROOT / 'docs/quality.md').read_text()
    baseline = json.loads(text.split(MARKER, 1)[1].split('```json\n', 1)[1].split('```', 1)[0])
    errors = regressions(current, baseline)
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    print('quality baseline: no structural or hygiene regression')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
