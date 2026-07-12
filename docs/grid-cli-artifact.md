# nano.rust Grid CLI Artifact

This archive contains every Rust CLI in the workspace, built on CMS EL9 and
smoke-tested in the `cmssw/el9:x86_64-grid` worker image. It has no ROOT or
CMSSW runtime dependency.

The archive layout is:

- `bin/`: `nano`, `nano-mcp`, `nano-ui`, and `nano-workflow`.
- `configs/`: reviewed run cards, sample catalogues, and correction data.
- `crates/nano-spec/examples/`: physics-facing TOML and ADL specifications.
- `manifest.json`: source commit, toolchain, build image, features, and SHA-256
  checksums.
- `verify.py`: artifact integrity, dynamic-linking, and launch checks.

Unpack the archive in the job sandbox and run tools from its root so relative
paths in analysis specs continue to resolve:

```bash
tar -xzf nano-rust-cli-el9-x86_64.tar.gz
cd nano-rust-cli-el9-x86_64
python3 verify.py .
./bin/nano validate --catalogue-version v15 \
  crates/nano-spec/examples/wz_vbs.toml
```

For HTCondor, transfer the `.tar.gz` file with the job and unpack it in the
wrapper before invoking a CLI. The artifact targets Linux x86-64 and is tested
against the official CMS EL9 grid image; rebuild it for another architecture or
OS baseline.
