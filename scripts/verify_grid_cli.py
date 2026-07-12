#!/usr/bin/env python3
"""Verify artifact integrity and launch every CLI through the host loader."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    binaries = manifest.get("binaries", [])
    if not binaries:
        raise SystemExit("manifest contains no binaries")

    for entry in binaries:
        binary = root / entry["path"]
        if not binary.is_file():
            raise SystemExit(f"missing binary: {binary}")
        if sha256(binary) != entry["sha256"]:
            raise SystemExit(f"checksum mismatch: {binary}")

        linked = subprocess.run(
            ["ldd", str(binary)], capture_output=True, text=True, check=False
        )
        link_output = linked.stdout + linked.stderr
        if "not found" in link_output:
            raise SystemExit(f"unresolved runtime library for {binary}:\n{link_output}")

        launched = subprocess.run(
            [str(binary), "--help"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=10,
            check=False,
        )
        if launched.returncode < 0 or launched.returncode in (126, 127):
            raise SystemExit(
                f"failed to launch {binary} (exit {launched.returncode}):\n{launched.stdout}"
            )
        print(f"OK {entry['name']}: linked and launchable (exit {launched.returncode})")

    config_root = root / manifest["config_root"]
    if not config_root.is_dir():
        raise SystemExit(f"missing config root: {config_root}")
    print(f"OK config root: {config_root}")

    spec_root = root / manifest["spec_root"]
    if not spec_root.is_dir():
        raise SystemExit(f"missing spec root: {spec_root}")
    print(f"OK spec root: {spec_root}")

    wz_spec = spec_root / "wz_vbs.toml"
    validation = subprocess.run(
        [
            str(root / "bin/nano"),
            "validate",
            "--catalogue-version",
            "v15",
            str(wz_spec.relative_to(root)),
        ],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=30,
        check=False,
    )
    if validation.returncode != 0:
        raise SystemExit(f"WZ spec validation failed:\n{validation.stdout}")
    print("OK WZ VBS spec and correction payload")


if __name__ == "__main__":
    main()
