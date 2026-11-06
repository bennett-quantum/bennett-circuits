#!/usr/bin/env python3
"""Resolve a gate.toml sibling pin, never falling back to a live checkout."""
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


def head(path):
    if not (path / ".git").exists():
        return "<unavailable>"
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    result = subprocess.run(["git", "-C", str(path), "rev-parse", "HEAD"],
                            capture_output=True, text=True, env=env, timeout=10)
    return result.stdout.strip() if result.returncode == 0 else "<unavailable>"


def resolve(root, name):
    root = Path(root).resolve()
    pin = tomllib.loads((root / "gate.toml").read_text())["siblings"][name]["sha"]
    if not re.fullmatch(r"[0-9a-f]{40}", pin):
        raise ValueError(f"{name}: gate.toml requires a full commit SHA, got {pin!r}")
    layout = os.environ.get("BENNETT_GATE_LAYOUT")
    path = (Path(layout).absolute() / name if layout is not None else
            root / f".tmp/gate/cache/{name}@{pin[:12]}/{name}")
    actual = head(path)
    if actual != pin:
        live = head(root.parent / name)
        raise ValueError(
            f"pinned sibling {name} at {pin}: {path} HEAD={actual}; "
            f"live ../{name} HEAD={live}; materialise with "
            f"gate_run.py verify --repo {root} --sha HEAD")
    return path


if __name__ == "__main__":
    try:
        print(resolve(Path(sys.argv[1]), sys.argv[2]))
    except (OSError, ValueError, KeyError, IndexError, subprocess.SubprocessError) as error:
        sys.exit(f"pinned sibling: {error}")
