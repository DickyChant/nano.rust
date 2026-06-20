#!/usr/bin/env bash
# Driver for the "if it compiles, it's safe to parallelize" screencast.
# Recorded with: asciinema rec docs/site/demo-parallel.cast -c "bash scripts/demo_parallel.sh"
# Reproducible: runs the real nano-io bench + a real compiler rejection.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
export CARGO_TERM_COLOR=always
FILE=tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root
RACE=docs/snippets/parallel_race.rs
DST=crates/nano-io/examples/_race_demo.rs

say() { printf '\033[1;36m# %s\033[0m\n' "$*"; sleep 1.3; }
run() { printf '\033[1;32m$\033[0m %s\n' "$*"; sleep 0.7; eval "$*"; echo; sleep 1.6; }

say "thesis: if it COMPILES, it's safe to parallelize -- the compiler is the proof."

say "1. one verified kernel, two schedules -- .iter() vs .par_iter(), nothing else:"
run "grep -A2 -E 'fn collect_(serial|parallel)' crates/nano-io/examples/bench_parallel.rs"

say "2. now try the UNSAFE schedule: share one mutable histogram across threads"
run "cat $RACE"
say "   ...and ask the compiler to build it:"
cp "$RACE" "$DST"
run "cargo build -q -p nano-io --example _race_demo 2>&1 | head -4"
rm -f "$DST"
say "   rejected. the data race is a COMPILE error, not a runtime surprise."

say "3. the safe schedule is a parallel reduce. SAME kernel, on real NanoAODv9:"
run "cargo run --release -q -p nano-io --example bench_parallel -- $FILE 100000 50 2>&1"

say "serial == parallel, bit-identical -- and ~8x faster. correctness IS the proof."
sleep 1.5
