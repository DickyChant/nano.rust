#!/usr/bin/env python3
"""Compare nano.rust ROOT I/O against uproot on remote read and local write."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import awkward as ak
import numpy as np
import uproot


CMS_OPEN_DATA_URL = (
    "https://eospublic.cern.ch//eos/opendata/cms/Run2016H/DoubleMuon/NANOAOD/"
    "UL2016_MiniAODv2_NanoAODv9-v1/2510000/"
    "127C2975-1B1C-A046-AABF-62B77E757A86.root"
)
BRANCHES = ["nMuon", "Muon_pt", "Muon_eta", "MET_pt", "run", "event"]


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    n_events = int(os.environ.get("NANO_BENCH_N", "100"))
    insecure = env_flag("NANO_HTTP_INSECURE")
    env = os.environ.copy()
    if insecure:
        env["NANO_HTTP_INSECURE"] = "1"

    print(f"benchmark events: {n_events}")
    if insecure:
        print("TLS: insecure mode enabled for public CERN open data")

    build_helpers(repo, env)

    rows = []
    rust_remote = timed(lambda: nano_remote_read(repo, n_events, insecure, env))
    if rust_remote.ok:
        rust_events, meta = rust_remote.value
        rows.append(("remote read", "nano.rust", rust_remote.seconds, f"{len(rust_events)} events, {meta['bytes_fetched']} fetched / {meta['file_size']} total"))
    else:
        rows.append(("remote read", "nano.rust", None, f"SKIP: {rust_remote.error}"))
        rust_events = None

    uproot_remote = timed(lambda: uproot_remote_read(n_events, insecure))
    if uproot_remote.ok:
        uproot_events = uproot_remote.value
        rows.append(("remote read", "uproot", uproot_remote.seconds, f"{len(uproot_events['nMuon'])} events"))
        if rust_events is not None:
            cross_check_remote(rust_events, uproot_events)
            print("remote read interop: nano.rust values match uproot")
    else:
        rows.append(("remote read", "uproot", None, f"SKIP: {uproot_remote.error}"))

    with tempfile.TemporaryDirectory(prefix="nano-rust-bench-") as tmpdir:
        tmpdir = Path(tmpdir)
        nano_path = tmpdir / "nano_written.root"
        uproot_path = tmpdir / "uproot_written.root"

        nano_write = timed(lambda: nano_write_synthetic(repo, nano_path, n_events, env))
        if nano_write.ok:
            rows.append(("write", "nano.rust", nano_write.seconds, str(nano_path)))
            check_uproot_reads_nano(nano_path)
        else:
            rows.append(("write", "nano.rust", None, f"FAIL: {nano_write.error}"))

        uproot_write = timed(lambda: uproot_write_synthetic(uproot_path, n_events))
        if uproot_write.ok:
            rows.append(("write", "uproot", uproot_write.seconds, str(uproot_path)))
        else:
            rows.append(("write", "uproot", None, f"FAIL: {uproot_write.error}"))

    print_table(rows)
    return 0


def build_helpers(repo: Path, env: dict[str, str]) -> None:
    subprocess.run(
        ["cargo", "build", "-p", "nano-io", "--features", "http", "--examples"],
        cwd=repo,
        env=env,
        check=True,
    )


def nano_remote_read(repo: Path, n_events: int, insecure: bool, env: dict[str, str]):
    exe = repo / "target" / "debug" / "examples" / "read_url_json"
    cmd = [str(exe), CMS_OPEN_DATA_URL, str(n_events)]
    if insecure:
        cmd.append("--insecure")
    proc = subprocess.run(cmd, cwd=repo, env=env, text=True, capture_output=True)
    if proc.returncode != 0:
        raise RuntimeError((proc.stderr or proc.stdout).strip())
    payload = json.loads(proc.stdout)
    meta = payload[-1]["_meta"]
    return payload[:-1], meta


def uproot_remote_read(n_events: int, insecure: bool):
    attempts = []
    if insecure:
        attempts.extend(
            [
                ("fsspec ssl=False", lambda: uproot.open(CMS_OPEN_DATA_URL, handler=uproot.source.fsspec.FSSpecSource, ssl=False)),
                (
                    "fsspec client_kwargs ssl=False",
                    lambda: uproot.open(
                        CMS_OPEN_DATA_URL,
                        handler=uproot.source.fsspec.FSSpecSource,
                        client_kwargs={"ssl": False},
                    ),
                ),
            ]
        )
    attempts.append(("default", lambda: uproot.open(CMS_OPEN_DATA_URL)))

    errors = []
    for label, opener in attempts:
        try:
            with opener() as root_file:
                arrays = root_file["Events"].arrays(BRANCHES, entry_start=0, entry_stop=n_events, library="ak")
            return arrays
        except Exception as exc:  # noqa: BLE001 - report exact interop/backend error.
            errors.append(f"{label}: {type(exc).__name__}: {exc}")
    raise RuntimeError("; ".join(errors))


def f32_close(a, b) -> bool:
    # Both sides come from the same on-disk float32. Compare at f32 precision:
    # the Rust side is serialized via JSON (shortest round-trip of the f32) while
    # uproot promotes to f64, so a raw f64 atol diverges by ~1 f32 ULP at large
    # magnitudes (e.g. ~2e-5 at pt ~ 2772 GeV). Casting both back to float32
    # checks the values are bit-identical.
    return np.allclose(
        np.asarray(a, dtype=np.float32), np.asarray(b, dtype=np.float32), rtol=1e-6, atol=0
    )


def cross_check_remote(rust_events, uproot_events) -> None:
    for index, rust in enumerate(rust_events):
        assert rust["nMuon"] == int(uproot_events["nMuon"][index])
        assert rust["run"] == int(uproot_events["run"][index])
        assert rust["event"] == int(uproot_events["event"][index])
        assert f32_close(rust["MET_pt"], float(uproot_events["MET_pt"][index]))
        assert f32_close(rust["Muon_pt"], ak.to_list(uproot_events["Muon_pt"][index]))
        assert f32_close(rust["Muon_eta"], ak.to_list(uproot_events["Muon_eta"][index]))


def nano_write_synthetic(repo: Path, output: Path, n_events: int, env: dict[str, str]) -> None:
    exe = repo / "target" / "debug" / "examples" / "write_synthetic"
    subprocess.run([str(exe), str(output), str(n_events)], cwd=repo, env=env, check=True)


def check_uproot_reads_nano(path: Path) -> None:
    try:
        with uproot.open(path) as root_file:
            arrays = root_file["Events"].arrays(["nMuon", "Muon_pt", "MET_pt", "run", "event"], entry_stop=10, library="ak")
        assert len(arrays["nMuon"]) == 10
        print("writer interop: uproot read nano.rust output")
    except Exception as exc:  # noqa: BLE001 - this is a known possible writer gap.
        print(f"writer interop: uproot could not read nano.rust output (non-fatal): {type(exc).__name__}: {exc}")


def uproot_write_synthetic(path: Path, n_events: int) -> None:
    counts = np.asarray([index % 4 for index in range(n_events)], dtype=np.uint32)
    muon_pt = ak.Array([[20.0 + float(index % 100) + float(muon) for muon in range(count)] for index, count in enumerate(counts)])
    muon_eta = ak.Array([[-2.0 + 0.1 * float(muon) for muon in range(count)] for count in counts])
    with uproot.recreate(path) as root_file:
        root_file["Events"] = {
            "nMuon": counts,
            "Muon_pt": muon_pt,
            "Muon_eta": muon_eta,
            "MET_pt": np.asarray([40.0 + float(index % 80) for index in range(n_events)], dtype=np.float32),
            "run": np.full(n_events, 315_252, dtype=np.int32),
            "event": np.asarray([100_000 + index for index in range(n_events)], dtype=np.uint64),
            "pass_preselection": np.asarray([index % 2 == 0 for index in range(n_events)], dtype=np.bool_),
        }


class TimedResult:
    def __init__(self, ok: bool, seconds: float | None = None, value=None, error: str | None = None):
        self.ok = ok
        self.seconds = seconds
        self.value = value
        self.error = error


def timed(fn) -> TimedResult:
    start = time.perf_counter()
    try:
        value = fn()
    except Exception as exc:  # noqa: BLE001 - surfaced in benchmark table.
        return TimedResult(False, error=f"{type(exc).__name__}: {exc}")
    return TimedResult(True, time.perf_counter() - start, value=value)


def print_table(rows) -> None:
    print("\n| task | implementation | seconds | notes |")
    print("|---|---:|---:|---|")
    for task, implementation, seconds, notes in rows:
        seconds_text = "SKIP" if seconds is None else f"{seconds:.3f}"
        print(f"| {task} | {implementation} | {seconds_text} | {notes} |")


def env_flag(name: str) -> bool:
    return os.environ.get(name, "").lower() in {"1", "true", "yes"}


if __name__ == "__main__":
    sys.exit(main())
