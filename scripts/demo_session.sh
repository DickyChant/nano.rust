#!/usr/bin/env bash
# Driver for the spec -> validate -> branches -> codegen -> proof demo.
# Recorded with: asciinema rec docs/site/demo.cast -c "bash scripts/demo_session.sh"
# Reproducible: it runs the real nano binary + cargo, no hand-edited frames.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
export PATH="$PWD/target/debug:$PATH"
export CARGO_TERM_COLOR=always
SPEC=crates/nano-spec/examples/muon.toml

say()  { printf '\033[1;36m# %s\033[0m\n' "$*"; sleep 1.2; }
run()  { printf '\033[1;32m$\033[0m %s\n' "$*"; sleep 0.7; eval "$*"; echo; sleep 1.6; }

say "1. the spec a physicist writes -- a muon control region, in TOML"
run "cat $SPEC"

say "2. validate: reject inconsistent physics BEFORE any I/O"
run "nano validate $SPEC"

say "3. derive the exact read set from the spec -- nothing 'just in case'"
run "nano branches $SPEC"

say "4. codegen: the spec becomes a readable, typed event loop"
run "nano codegen $SPEC | head -40"

say "5. proof: the generated loop == the hand-written reference (CI gate)"
run "cargo test -p nano-gen-demo 2>&1 | grep -E 'matches_handwritten|test result' | head -3"

say "6. now a WRONG analysis: 'etaa' is a typo for 'eta'"
run "cat crates/nano-spec/examples/muon_broken.toml | sed -n '8,11p'"

say "   in a dynamic framework this silently reads garbage. here:"
run "nano validate crates/nano-spec/examples/muon_broken.toml || echo \"   (rejected, exit \$?)\""

say "good spec -> code + proof. bad spec -> blocked before any I/O. done."
sleep 1.5
