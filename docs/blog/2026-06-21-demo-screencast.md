# Watch it: spec to code, and a wrong spec getting blocked

*2026-06-21 — a 27-second terminal screencast of the real workflow, plus the
case that matters most: a wrongly-defined analysis being rejected before it can
produce a wrong number.*

The [companion note](2026-06-21-spec-to-code.html) walks through the artifacts in
detail. This one is just the screencast — the live binaries, no hand-edited
frames. It runs the four steps end to end and then does the thing soft
guardrails can't: it feeds the validator a *broken* spec and watches it say no.

<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.css">
<div id="player" style="margin:1.5rem 0"></div>
<script src="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.min.js"></script>
<script>
  AsciinemaPlayer.create('../demo.cast', document.getElementById('player'), {
    cols: 80, rows: 24, idleTimeLimit: 2, theme: 'asciinema', poster: 'npt:0:3'
  });
</script>

*(If the player doesn't load, the raw cast is at
[`demo.cast`](../demo.cast); play it with `asciinema play demo.cast`.)*

## The good path (steps 1–5 in the screencast)

```text
nano validate  spec.toml   # reject inconsistent physics before any I/O
nano branches  spec.toml   # derive the exact read set (nothing "just in case")
nano codegen   spec.toml   # emit a readable, typed event loop
cargo test -p nano-gen-demo   # prove the generated loop == the trusted reference
```

That chain — spec → validated plan → generated kernel → proven equivalent to the
hand-written reference — is the whole "agents write, humans review" contract made
mechanical.

## The bad path (step 6) — the point of the whole exercise

The last step feeds in `crates/nano-spec/examples/muon_broken.toml`, which has a
single fat-fingered character:

```toml
cuts = ["pt > 30 GeV", "abs(etaa) < 2.4"]   # <-- typo: Muon_etaa does not exist
```

In a dynamic, stringly-typed framework (the Python/`TTreeReader` status quo) this
is the *worst* kind of bug: it doesn't crash. The branch lookup quietly returns
nothing, the cut silently does the wrong thing, and you get a plausible-looking
number that is wrong. Nobody sees a stack trace; a reviewer has to *notice* the
extra letter.

Here it is a hard stop:

```console
$ nano validate crates/nano-spec/examples/muon_broken.toml
Validation: spec validation failed
  - object `good_muon` cut 2: missing branch `Muon_etaa`
(rejected, exit 1)
```

The validator resolves every branch the spec touches against the NanoAODv9
catalogue, so `Muon_etaa` is rejected with a precise message and a non-zero exit
code — which means **CI fails the build**. The other failure modes are caught the
same way and are easy to reproduce:

| Broken spec | What validation says |
|---|---|
| `abs(etaa) < 2.4` (typo'd attribute) | `cut 2: missing branch Muon_etaa` |
| `pt > 30` (unit dropped) | `cut 1: comparison is missing required unit GeV` |
| `count(good_electron) >= 1` (undefined object) | `requirement 1: undefined object good_electron` |

A dropped `GeV`, an object that was never defined, a branch that doesn't exist
for this era — none of them reach the event loop. This is the thesis in one
screen: correctness lives in the validator/compiler, not in a reviewer's
attention.

## Reproduce the screencast

Everything is in the repo; the recording is regenerable, not a one-off capture:

```console
$ cargo build -p nano-cli
$ asciinema rec docs/site/demo.cast --overwrite -c "bash scripts/demo_session.sh"
```

`scripts/demo_session.sh` just runs the real `nano` binary and `cargo test` with
a bit of pacing — same spirit as the rest of the project: if it's on screen, it
actually ran.
