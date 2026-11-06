#!/usr/bin/env python3
"""Run sibling-reading test binaries with an empty live sibling directory.

The temporary consumer has this tree's gate cache, or only a gate layout with
the pinned checkout. No live sibling source is copied into the workspace.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--layout", action="store_true")
    parser.add_argument("tests", nargs="*", help="test binaries (default: all sibling readers)")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    pins = tomllib.loads((root / "gate.toml").read_text())["siblings"]
    readers = args.tests or sorted(
        p.stem for p in (root / "tests").glob("*.rs")
        if p.stem == "pinned_siblings" or "bennett-isa-spec" in p.read_text() or "sibling::" in p.read_text()
    )
    assert readers, "no sibling-reading test binaries discovered"
    with tempfile.TemporaryDirectory(prefix="pin-readers-") as tmp:
        workspace = Path(tmp)
        consumer = workspace / "bennett-circuits"
        consumer.mkdir()
        for name in ("Cargo.toml", "SPEC_VERSIONS.toml", "gate.toml"):
            shutil.copyfile(root / name, consumer / name)
        for name in ("src", "tests", "scripts"):
            # Fresh mtimes force Cargo to rebuild env!("CARGO_MANIFEST_DIR") for
            # this temporary path when reusing the job's CARGO_TARGET_DIR.
            shutil.copytree(root / name, consumer / name, copy_function=shutil.copy)
        env = os.environ.copy()
        env.pop("BENNETT_GATE_LAYOUT", None)
        if args.layout:
            layout = workspace / "layout"
            layout.mkdir()
            env["BENNETT_GATE_LAYOUT"] = str(layout)
        for name, pin in pins.items():
            (workspace / name).mkdir()  # Deliberately empty live checkout.
            checkout = Path(subprocess.check_output(
                [sys.executable, "-I", str(root / "scripts/pinned_sibling.py"), str(root), name],
                text=True, timeout=15,
            ).strip())
            if args.layout:
                (layout / name).symlink_to(checkout.resolve(), target_is_directory=True)
            else:
                cache = consumer / f".tmp/gate/cache/{name}@{pin['sha'][:12]}/{name}"
                cache.parent.mkdir(parents=True)
                cache.symlink_to(checkout.resolve(), target_is_directory=True)
        command = ["cargo", "test", "--offline", "--no-fail-fast", "-j3", "--manifest-path", str(consumer / "Cargo.toml")]
        # Temporary manifests must not overwrite binaries for the real checkout:
        # Cargo can otherwise reuse their embedded, now-deleted manifest paths.
        if target_dir := env.get("CARGO_TARGET_DIR"):
            env["CARGO_TARGET_DIR"] = str(Path(target_dir).resolve() / "pin-readers")
        for reader in readers:
            command.extend(["--test", reader])
        command.extend(["--", "--test-threads=4"])
        print(f"Empty live siblings; {'layout' if args.layout else 'cache'} pins; readers: {', '.join(readers)}", flush=True)
        return subprocess.run(command, env=env, timeout=900).returncode


if __name__ == "__main__":
    raise SystemExit(main())
