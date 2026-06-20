# nano.rust

[![CI](https://github.com/DickyChant/nano.rust/actions/workflows/ci.yml/badge.svg)](https://github.com/DickyChant/nano.rust/actions/workflows/ci.yml)
[![docs](https://github.com/DickyChant/nano.rust/actions/workflows/docs.yml/badge.svg)](https://github.com/DickyChant/nano.rust/actions/workflows/docs.yml)
[![links](https://github.com/DickyChant/nano.rust/actions/workflows/links.yml/badge.svg)](https://github.com/DickyChant/nano.rust/actions/workflows/links.yml)

A **pure-Rust, semantics-first NanoAOD analysis framework** for high-energy physics,
built for the agentic-coding era.

📖 **API docs + notes:** https://dickychant.github.io/nano.rust/

## The idea

Agentic tools can write analysis code faster than anyone can review it. The
bottleneck is no longer *writing* code but *guaranteeing* it is correct. Soft
guardrails — prompts, skills, harnesses, human review — can *steer* an agent but
cannot *guarantee* the absence of silent analysis bugs (wrong branch, dropped
systematic, mixed units, stale outputs). A hard guarantee needs a mechanical
enforcer:

> Physicists define and review physics **semantics**.
> Agents generate **implementation**.
> The **Rust compiler** and a validation layer reject inconsistent states.

So the analysis is modelled as a typed **state machine** the compiler checks
(make invalid analysis states unrepresentable), with Rust's strengths layered on:
performance, FFI to legacy libraries, SIMD per-event execution, and TUI-friendly
orchestration. Full rationale: [`docs/vision.md`](docs/vision.md).

## What works today

- **Owned, pure-Rust ROOT I/O** (`nano-rootio`, no ROOT/C++ dependency):
  - reads real **CMS NanoAODv9** — scalars, jagged collections, windowed reads,
    **bounded-memory streaming** (~3 MB to stream a skim of any-size file);
  - reads **locally and remotely on-demand** over HTTPS byte-range (the first 10
    events of a 2 GB open-data file fetch ~1.3 MB — only the baskets touched);
  - **writes** ROOT/uproot-readable skims (scalars **and** jagged);
  - validated A/B against the upstream reader and cross-checked against
    **`uproot`** in CI, both read and write.
- **Typed event model** (`nano-core`): collections, attributes, the
  `Prefix_attr` grouping rule, `Rc`-shared per-event columns.
- **Compile-enforced state machine** (`nano-analysis`):
  `Ev<Raw> → Baseline → InRegion<R> → Weighted<R>`; filling a histogram
  *requires* a `Weighted<R>`, so wrong-stage / wrong-region / unweighted fills are
  **compile errors** (proven by compile-fail tests). Unit newtypes, exhaustive
  `Systematic`.
- **Semantic IR** (`nano-spec`): a physics-facing YAML spec is parsed, statically
  validated (missing branch / wrong type / missing unit / undefined object are
  rejected with precise errors), and used to **derive the exact `read_branches`**
  for the reader.

## Workspace

```
crates/
  nano-rootio    owned ROOT TTree read + write (NanoAOD subset; pure Rust)
  nano-core      event model (Event / Collection / ObjectView, branch schema)
  nano-io        streaming reader + skim writer over nano-rootio
  nano-producers analysis channels (muon control region)
  nano-analysis  compile-enforced analysis state machine (typestate)
  nano-spec      semantic compiler: spec -> validate -> derive read_branches -> codegen
  nano-corrections  native correctionlib evaluator (typed SF inputs)
  nano-inference    backend-agnostic ML inference protocol (mock/ONNX/remote/managed)
  nano-cli       the `nano` CLI: validate / branches / inspect / codegen
  nano-mcp       MCP server exposing the same ops as agent tools
  nano-gen-demo, nano-gen-tagger-demo   codegen == hand-written equivalence proofs
  root-io        vendored upstream reader, retained only as a dev/A-B oracle
```

The architecture, layer by layer:

```
physics spec (TOML/YAML)  ->  semantic IR (typed, validated) -> Rust codegen
                          ->  Rust execution kernels (typed state machine)
                          ->  Rust-native workflow DAG (planned)
```

## Build, test, run

```bash
cargo build
cargo test                 # whole workspace
cargo test --features http # also exercise remote (HTTPS byte-range) reads

# write a small NanoAOD-like file and inspect it (e.g. with uproot)
cargo run -p nano-rootio --example write_demo -- /tmp/demo.root
```

Real-data tests read a local NanoAOD file from
`tests/data/muon_validation/inputs/` if present (gitignored) and skip otherwise.
The `uproot` interop + benchmark runs in CI (`scripts/bench_vs_uproot.py`)
against CMS Open Data over HTTPS — **no checked-in data files**.

## Status & roadmap

Built: owned ROOT I/O (read + write, local + remote), the event model, the
compile-enforced state machine, the semantic compiler **including codegen**
(proven equal to a hand-written producer), a **native `correctionlib`**
evaluator, an **ML inference protocol**, and an agent action space (`nano` CLI +
MCP server). Next: golden tests against the frozen `.root` references, wiring
real corrections/JME systematics into the channel, and a **Rust-native workflow
DAG orchestrator** (the LAW backend is descoped). See [`docs/`](docs/) —
[vision](docs/vision.md), [versioning](docs/versioning.md),
[state machine](docs/state-machine.md), [semantic layer](docs/semantic-layer.md),
[inference protocol](docs/inference-protocol.md),
[agent interface](docs/agent-interface.md),
[reader rewrite](docs/reader-rewrite.md), [remote source](docs/xrootd-source.md),
[migration](docs/rust-migration.md).

## Acknowledgments

nano.rust grew out of, and is inspired by, prior work:

- **Origins** — it began as a C++ port (`nano.cpp` / NanoAODToolsCpp) of selected
  [NanoAOD-tools](https://github.com/cms-nanoAOD/nanoAOD-tools) /
  [NanoHRT-tools](https://github.com/hqucms/NanoHRT-tools) workflows, preserved on
  the `cpp-snapshot` branch.
- **[root-io](https://github.com/cbourjau/alice-rs)** (cbourjau / alice-rs) — the
  pure-Rust ROOT reader we vendored and grew the owned `nano-rootio` I/O core
  (read + write) from; still a differential A/B oracle in tests (MPL-2.0).
- **[uproot](https://github.com/scikit-hep/uproot5)** (with awkward-array) — for
  showing that ROOT can be treated as a *storage format* readable outside ROOT;
  it is also our independent read/write oracle in CI.
- **[ROOT](https://root.cern/)** — the on-disk format and reference
  implementation; our correctness and performance baseline.
- **[correctionlib](https://github.com/cms-nanoAOD/correctionlib)** — the
  corrections JSON schema and evaluation model that `nano-corrections`
  re-implements natively in Rust.

## License

MPL-2.0. `crates/root-io` is vendored from
[`cbourjau/alice-rs`](https://github.com/cbourjau/alice-rs) (MPL-2.0); its license
and attribution are retained.
