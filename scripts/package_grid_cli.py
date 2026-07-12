#!/usr/bin/env python3
"""Assemble all workspace CLI binaries into a self-describing grid artifact."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess


def command_output(*args: str) -> str:
    return subprocess.check_output(args, text=True).strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def cli_targets() -> list[dict[str, str]]:
    metadata = json.loads(
        command_output("cargo", "metadata", "--no-deps", "--format-version", "1")
    )
    targets = []
    for package in metadata["packages"]:
        if package["id"] not in metadata["workspace_members"]:
            continue
        for target in package["targets"]:
            if "bin" in target["kind"]:
                targets.append({"name": target["name"], "package": package["name"]})
    return sorted(targets, key=lambda target: target["name"])


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target-dir", type=Path, default=Path("target/release"))
    parser.add_argument(
        "--output-dir", type=Path, default=Path("dist/nano-rust-cli-el9-x86_64")
    )
    parser.add_argument("--build-image", required=True)
    parser.add_argument("--features", default="")
    parser.add_argument("--repository")
    parser.add_argument("--commit")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    targets = cli_targets()
    if not targets:
        raise SystemExit("cargo metadata did not report any CLI binary targets")

    missing = [
        target["name"]
        for target in targets
        if not (args.target_dir / target["name"]).is_file()
    ]
    if missing:
        raise SystemExit(f"release binaries are missing: {', '.join(missing)}")

    shutil.rmtree(args.output_dir, ignore_errors=True)
    bin_dir = args.output_dir / "bin"
    bin_dir.mkdir(parents=True)

    binaries = []
    for target in targets:
        source = args.target_dir / target["name"]
        destination = bin_dir / target["name"]
        shutil.copy2(source, destination)
        destination.chmod(0o755)
        binaries.append(
            {
                **target,
                "path": f"bin/{target['name']}",
                "bytes": destination.stat().st_size,
                "sha256": sha256(destination),
            }
        )

    shutil.copytree("configs", args.output_dir / "configs")
    spec_root = args.output_dir / "crates/nano-spec/examples"
    spec_root.parent.mkdir(parents=True)
    shutil.copytree("crates/nano-spec/examples", spec_root)
    shutil.copy2("docs/grid-cli-artifact.md", args.output_dir / "README.md")
    shutil.copy2("scripts/verify_grid_cli.py", args.output_dir / "verify.py")
    if Path("LICENSE").is_file():
        shutil.copy2("LICENSE", args.output_dir / "LICENSE")

    image_info = Path("/image-build-info.txt")
    repository = args.repository or command_output(
        "git", "config", "--get", "remote.origin.url"
    )
    commit = args.commit or command_output("git", "rev-parse", "HEAD")
    manifest = {
        "schema_version": 1,
        "source": {
            "repository": repository,
            "commit": commit,
        },
        "build": {
            "image": args.build_image,
            "image_info": image_info.read_text().strip()
            if image_info.is_file()
            else None,
            "architecture": platform.machine(),
            "rustc": command_output("rustc", "--version"),
            "features": [feature for feature in args.features.split(",") if feature],
        },
        "binaries": binaries,
        "config_root": "configs",
        "spec_root": "crates/nano-spec/examples",
    }
    (args.output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + os.linesep, encoding="utf-8"
    )

    print(f"packaged {len(binaries)} CLI binaries in {args.output_dir}")
    for binary in binaries:
        print(f"  {binary['name']} ({binary['package']}, {binary['bytes']} bytes)")


if __name__ == "__main__":
    main()
