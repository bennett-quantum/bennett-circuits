"""Exercise the local conformance command boundary without rebuilding the ISA harness."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import tomllib

ROOT = Path(__file__).resolve().parent.parent


class ExporterGate(unittest.TestCase):
    def setUp(self):
        (ROOT / '.tmp').mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(prefix='export gate ', dir=ROOT / '.tmp')
        self.addCleanup(self.directory.cleanup)
        self.repo = Path(self.directory.name)
        (self.repo / 'scripts').mkdir()
        (self.repo / 'bin').mkdir()
        shutil.copyfile(ROOT / 'scripts/run-conformance', self.repo / 'scripts/run-conformance')
        (self.repo / 'Cargo.toml').write_text('[package]\nname="probe"\nversion="0.1.0"\n')
        (self.repo / 'scripts/pinned_sibling.py').write_text(
            'from pathlib import Path\nimport sys\nprint(Path(sys.argv[1])/"pin")\n')
        self.trace = self.repo / 'trace.jsonl'
        harness = '''import json,os,sys
with open(os.environ['TRACE'],'a') as log: print(json.dumps(['F',sys.argv[1:]]), file=log)
print('published-F-stdout')
sys.exit(int(os.environ['F_EXIT']))
'''
        rustc = '''import json,os,pathlib,sys
args=sys.argv[1:]
with open(os.environ['TRACE'],'a') as log: print(json.dumps(['rustc',args]), file=log)
if '-o' in args:
    path=pathlib.Path(args[args.index('-o')+1]); path.write_text(SHEBANG+HARNESS); path.chmod(0o755)
'''.replace('SHEBANG', repr('#!' + sys.executable + '\n')).replace('HARNESS', repr(harness))
        cargo = '''import json,os,sys,time
with open(os.environ['TRACE'],'a') as log: print(json.dumps(['cargo',sys.argv[1:]]), file=log)
if 'test' in sys.argv:
    time.sleep(float(os.environ.get('EXPORT_SLEEP','0')))
    print('local-exporter-stdout')
    sys.exit(int(os.environ['EXPORT_EXIT']))
'''
        for name, body in [('rustc', rustc), ('cargo', cargo)]:
            path = self.repo / 'bin' / name
            path.write_text('#!' + sys.executable + '\n' + body)
            path.chmod(0o755)

    def run_gate(self, f_status, export_status, **extra):
        env = dict(os.environ, PATH=str(self.repo / 'bin') + os.pathsep + os.environ['PATH'],
                   TRACE=str(self.trace), F_EXIT=str(f_status), EXPORT_EXIT=str(export_status),
                   TMPDIR=str(self.repo / '.tmp'), CARGO_TARGET_DIR=str(self.repo / 'target'))
        env.update(extra)
        result = subprocess.run(['bash', str(self.repo / 'scripts/run-conformance'), '--report', 'report.json'],
                                env=env, text=True, capture_output=True, timeout=30)
        events = [json.loads(line) for line in self.trace.read_text().splitlines()]
        return result, events

    def test_published_f_and_local_exporter_verdicts_are_distinct(self):
        result, events = self.run_gate(0, 0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'published-F-stdout\n')
        f = next(args for name, args in events if name == 'F')
        self.assertEqual(f, ['--cases', str(self.repo / 'pin/conformance/cases'), '--suite', 'F',
                             '--bin', str(self.repo / 'target/debug/bennett-circuits'), '--report', 'report.json'])
        local = [args for name, args in events if name == 'cargo' and args[0] == 'test']
        self.assertEqual(len(local), 1)
        self.assertIn('integration', local[0])
        self.assertIn('export_oracle::', local[0])
        self.assertNotIn('--ignored', local[0])
        self.assertIn('local-exporter-stdout', result.stderr)
        self.assertLess([name for name, _ in events].index('F'),
                        next(i for i, (name, args) in enumerate(events) if name == 'cargo' and args[0] == 'test'))

    def test_local_failure_cannot_be_masked_by_published_f(self):
        result, events = self.run_gate(0, 101)
        self.assertEqual(result.returncode, 101, result.stderr)
        self.assertTrue(any(name == 'cargo' and args[0] == 'test' for name, args in events))

    def test_published_failure_keeps_its_verdict_and_local_still_runs(self):
        result, events = self.run_gate(7, 101)
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertTrue(any(name == 'cargo' and args[0] == 'test' for name, args in events))


    def test_published_unavailable_is_not_relabelled_by_local_success(self):
        result, events = self.run_gate(2, 0)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn('Local exporter differential: PASS', result.stderr)
        self.assertTrue(any(name == 'cargo' and args[0] == 'test' for name, args in events))

    def test_hang_guard_is_unknown_not_a_passing_differential(self):
        result, _ = self.run_gate(0, 0, EXPORT_SLEEP='2', BENNETT_EXPORTER_GUARD_S='0.1')
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertIn('UNKNOWN', result.stderr)
        self.assertNotIn('Local exporter differential: PASS', result.stderr)

    def test_normal_gate_keeps_the_unfiltered_integration_suite(self):
        gate = tomllib.loads((ROOT / 'gate.toml').read_text())
        check = next(c for c in gate['check'] if c['name'] == 'cargo test')
        self.assertNotEqual(check.get('tier'), 'post-merge')
        argv = check['argv']
        self.assertEqual(argv[:2], ['cargo', 'test'])
        for selection in ['--lib', '--bin', '--bins', '--doc', '--example', '--test', '--ignored', '--skip', '--exact']:
            self.assertNotIn(selection, argv)
        tail = argv[argv.index('--') + 1:]
        self.assertTrue(all(a.startswith('--') for a in tail), tail)
        cargo = tomllib.loads((ROOT / 'Cargo.toml').read_text())
        self.assertTrue(any(t['name'] == 'integration' and t['path'] == 'tests/integration.rs' for t in cargo['test']))


if __name__ == '__main__':
    unittest.main()
