"""Pin resolver and conformance-script regressions; no sibling writes."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).with_name("pinned_sibling.py")
SPEC = importlib.util.spec_from_file_location("pinned_sibling", SCRIPT)
resolver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(resolver)
PIN = "a" * 40
LIVE = "b" * 40
NAME = "bennett-isa-spec"


def checkout(path, sha):
    # A minimal detached Git directory, sufficient for read-only rev-parse HEAD.
    (path / ".git/objects").mkdir(parents=True)
    (path / ".git/refs").mkdir()
    (path / ".git/HEAD").write_text(sha + "\n")
    return path


class PinFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="pin-resolver-")
        self.addCleanup(self.tmp.cleanup)
        self.workspace = Path(self.tmp.name)
        self.root = self.workspace / "consumer with spaces"
        self.root.mkdir()
        (self.root / "gate.toml").write_text(
            f'repo = "consumer"\n[siblings.other]\nsha = "{LIVE}"\n'
            f'[siblings."{NAME}"]\nsha = "{PIN}"\n'
        )
        self.cache = self.root / f".tmp/gate/cache/{NAME}@{PIN[:12]}/{NAME}"
        self.live = checkout(self.workspace / NAME, LIVE)
        self.env = patch.dict(os.environ)
        self.env.start()
        self.addCleanup(self.env.stop)
        os.environ.pop("BENNETT_GATE_LAYOUT", None)

    def run_cli(self):
        return subprocess.run(
            [sys.executable, "-I", str(SCRIPT), str(self.root), NAME],
            capture_output=True, text=True, timeout=10,
        )

    def assert_failure(self, output):
        self.assertEqual(output.returncode, 1)
        self.assertEqual(output.stdout, "")
        self.assertEqual(len(output.stderr.splitlines()), 1)
        for text in (NAME, PIN, f"live ../{NAME} HEAD={LIVE}",
                     f"gate_run.py verify --repo {self.root} --sha HEAD"):
            self.assertIn(text, output.stderr)


class PinResolution(PinFixture):
    def test_cache_wins_over_live_checkout(self):
        checkout(self.cache, PIN)
        output = self.run_cli()
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertEqual(Path(output.stdout.strip()), self.cache)
        self.assertEqual(output.stderr, "")

    def test_layout_wins_over_wrong_cache_and_live_checkout(self):
        checkout(self.cache, LIVE)
        layout = self.workspace / "gate layout"
        pinned = checkout(layout / NAME, PIN)
        os.environ["BENNETT_GATE_LAYOUT"] = str(layout)
        self.assertEqual(resolver.resolve(self.root, NAME), pinned)

    def test_missing_cache_reports_pin_and_actual_live_head_once(self):
        self.assert_failure(self.run_cli())

    def test_live_at_the_pin_is_still_not_a_cache_fallback(self):
        (self.live / ".git/HEAD").write_text(PIN + "\n")
        output = self.run_cli()
        self.assertEqual(output.returncode, 1)
        self.assertEqual(output.stdout, "")
        self.assertEqual(len(output.stderr.splitlines()), 1)
        self.assertIn(f"live ../{NAME} HEAD={PIN}", output.stderr)
        self.assertIn(f"{self.cache} HEAD=<unavailable>", output.stderr)

    def test_missing_layout_does_not_fall_back_to_cache(self):
        checkout(self.cache, PIN)
        os.environ["BENNETT_GATE_LAYOUT"] = str(self.workspace / "missing layout")
        self.assert_failure(self.run_cli())

    def test_wrong_cache_head_is_rejected(self):
        checkout(self.cache, LIVE)
        output = self.run_cli()
        self.assert_failure(output)
        self.assertIn(f"{self.cache} HEAD={LIVE}", output.stderr)

    def test_wrong_layout_head_does_not_fall_back_to_cache(self):
        checkout(self.cache, PIN)
        layout = self.workspace / "layout"
        checkout(layout / NAME, LIVE)
        os.environ["BENNETT_GATE_LAYOUT"] = str(layout)
        self.assert_failure(self.run_cli())

    def test_empty_directory_cannot_inherit_consumers_git_head(self):
        # git -C inside a plain subdirectory would otherwise find the parent's HEAD.
        checkout(self.root, PIN)
        self.cache.mkdir(parents=True)
        self.assert_failure(self.run_cli())

    def test_bad_pin_is_loud(self):
        (self.root / "gate.toml").write_text(f'[siblings."{NAME}"]\nsha = "aaaa"\n')
        output = self.run_cli()
        self.assertEqual(output.returncode, 1)
        self.assertEqual(len(output.stderr.splitlines()), 1)
        self.assertIn("full commit SHA", output.stderr)


class ConformanceScript(PinFixture):
    def setUp(self):
        super().setUp()
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("run-conformance", "pinned_sibling.py"):
            (scripts / name).write_text(SCRIPT.with_name(name).read_text())
        self.bin = self.workspace / "bin"
        self.bin.mkdir()
        self.log = self.workspace / "commands.jsonl"
        # Capture compilation and execution without building a sibling harness.
        stub = '''#!/usr/bin/env python3
import json, os, pathlib, sys
with open(os.environ["PIN_COMMAND_LOG"], "a") as log:
    log.write(json.dumps(sys.argv) + "\\n")
if "-o" in sys.argv:
    output = pathlib.Path(sys.argv[sys.argv.index("-o") + 1])
    output.write_text(pathlib.Path(__file__).read_text())
    output.chmod(0o755)
'''
        for name in ("rustc", "cargo"):
            tool = self.bin / name
            tool.write_text(stub)
            tool.chmod(0o755)
        os.environ["PATH"] = str(self.bin) + os.pathsep + os.environ["PATH"]
        os.environ["PIN_COMMAND_LOG"] = str(self.log)
        os.environ["CARGO_TARGET_DIR"] = str(self.root / ".tmp/custom-target")

    def run_script(self):
        return subprocess.run(
            ["bash", str(self.root / "scripts/run-conformance"), "--verbose"],
            capture_output=True, text=True, timeout=10,
        )

    def test_script_compiles_only_pinned_harness_and_uses_target_dir(self):
        checkout(self.cache, PIN)
        output = self.run_script()
        self.assertEqual(output.returncode, 0, output.stderr)
        commands = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(commands), 4)
        self.assertIn(str(self.cache / "conformance/src/lib.rs"), commands[0])
        self.assertIn(str(self.cache / "conformance/src/main.rs"), commands[1])
        self.assertIn(str(self.cache / "conformance/cases"), commands[3])
        self.assertIn(str(self.root / ".tmp/custom-target/debug/bennett-circuits"), commands[3])
        self.assertEqual(commands[3][-1], "--verbose")
        for args in commands:
            self.assertFalse(any(str(self.live) in arg for arg in args))

    def test_script_missing_pin_fails_before_compilation(self):
        self.assert_failure(self.run_script())
        self.assertFalse(self.log.exists(), "compiler ran despite a missing pin")


if __name__ == "__main__":
    unittest.main()
