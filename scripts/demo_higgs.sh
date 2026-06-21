#!/usr/bin/env bash
# nano.rust reconstructs the Higgs (df103 H->ZZ->4l) on CMS Open Data, bit-identical to ROOT.
# Recorded with: asciinema rec docs/site/demo-higgs.cast -c "bash scripts/demo_higgs.sh"
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
SIG="https://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/SMHiggsToZZTo4L.root"
say() { printf '\033[1;36m# %s\033[0m\n' "$*"; sleep 1.3; }
run() { printf '\033[1;32m$\033[0m %s\n' "$*"; sleep 0.7; eval "$*"; echo; sleep 1.6; }

say "ROOT's df103 Higgs->ZZ->4l analysis, ported to nano.rust, on CMS Open Data:"
say "3 channels (4mu/4e/2e2mu), lepton selection + Z pairing + 4-lepton mass."
run "target/debug/examples/higgs4l_opendata '$SIG' --insecure"
say "the 4-lepton mass peaks at 125 GeV -- the Higgs. read over HTTPS, pure Rust."
say "and it matches ROOT's df103 EXACTLY: total 26708, peak bin 23370 -- bit-identical."
say "with --plot it renders the peak via kuva (plot in the post)."
sleep 1.5
