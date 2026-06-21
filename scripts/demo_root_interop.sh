#!/usr/bin/env bash
# nano.rust reads ROOT's df102 NanoAOD tutorial file over HTTPS and agrees with ROOT.
# Recorded with: asciinema rec docs/site/demo-root-interop.cast -c "bash scripts/demo_root_interop.sh"
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
URL="https://eospublic.cern.ch//eos/opendata/cms/derived-data/AOD2NanoAODOutreachTool/Run2012BC_DoubleMuParked_Muons.root"
say() { printf '\033[1;36m# %s\033[0m\n' "$*"; sleep 1.3; }
run() { printf '\033[1;32m$\033[0m %s\n' "$*"; sleep 0.7; eval "$*"; echo; sleep 1.5; }

say "ROOT's df102 dimuon tutorial file -- inspect it over HTTPS, no download:"
run "target/debug/nano inspect '$URL' --insecure | head -7"
say "reproduce df102: pair opposite-charge muons, compute the dimuon mass:"
run "target/debug/examples/dimuon_opendata '$URL' 20000 --insecure | tail -20"
say "the Z peak is in the 80-100 GeV bin; ~0.03% of the 2 GB file fetched."
say "and the first events match ROOT (xrootd) bit-for-bit -- see the blog table."
sleep 1.5
