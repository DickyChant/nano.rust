#!/usr/bin/env bash
# Read real CMS Open Data over HTTPS, on demand, with no stored files.
# Recorded with: asciinema rec docs/site/demo-opendata.cast -c "bash scripts/demo_opendata.sh"
# (uses the prebuilt example binary so the demo doesn't depend on a compile)
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
URL="https://eospublic.cern.ch//eos/opendata/cms/Run2016H/DoubleMuon/NANOAOD/UL2016_MiniAODv2_NanoAODv9-v1/2510000/127C2975-1B1C-A046-AABF-62B77E757A86.root"
BIN=target/debug/examples/read_url_json

say() { printf '\033[1;36m# %s\033[0m\n' "$*"; sleep 1.3; }
run() { printf '\033[1;32m$\033[0m %s\n' "$*"; sleep 0.7; eval "$*"; echo; sleep 1.4; }

say "a ~2 GB CMS Open Data NanoAOD file on the CERN open-data server:"
run "echo '/eos/opendata/cms/Run2016H/DoubleMuon/.../127C2975-....root'"
say "read its first 5 events over HTTPS -- pure Rust, no ROOT, no download:"
run "$BIN \"\$URL\" 5 --insecure | python3 scripts/fmt_opendata.py"
say "only the baskets we touched were fetched (see _meta): ~1.3 MB of ~2 GB,"
say "~0.07% of the file. on-demand remote read, validated against uproot in CI."
sleep 1.5
