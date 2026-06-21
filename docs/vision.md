# nano.rust — North-Star Vision: a Semantics-First Analysis Framework

This is the long-horizon vision that the current C++→Rust port (`docs/rust-migration.md`)
is the **first phase** of. The migration plan is the *how-now*; this is the *where-to*.

## Thesis

Treat a HEP analysis not as a pile of scripts but as a **typed semantic object**:
datasets, branches, objects, selections, corrections, weights, systematics,
histograms, statistical models, workflow tasks, and outputs all carry explicit
types, provenance, and dependencies.

> Physicists define and review physics **semantics**.
> Agents generate **implementation**.
> The compiler and a validation system reject inconsistent software and workflow states.

Design principle: **make invalid analysis states unrepresentable.** Rust cannot catch a
wrong signal-region threshold, but it can catch wrong branch access, dropped systematic
variations, mixed units, stale outputs, and incomplete workflow dependencies — exactly
the error classes that agentic codegen produces and that today rely on manual review.

## Architecture (four layers)

```text
Physics-facing specification     (ADL/YAML/TOML/DSL — what physicists review)
        ↓
Analysis Semantic IR             (typed meaning; static validation against data + corrections)
        ↓
Rust execution kernels           (safe, fast event processing — generated, not hand-written)
        ↓
Workflow orchestration IR        (Rust-native typed DAG: chunking, provenance, staleness)
```

ROOT/correctionlib/Combine/pyhf are **boundary integrations**. ROOT is a *storage
format*, not the semantic core — no `TTree`/`TH1`/raw branch strings leak above the I/O
layer. The analysis *meaning* lives in the typed IR, independent of any backend.
(The workflow layer is **Rust-native**: the LAW backend is descoped — see below.)

## Where we are now (status against the roadmap)

| Phase | Scope | Status |
|---|---|---|
| 0 — Design study | IRs, spec syntax, design docs | **Done** — vision + migration + state-machine + semantic-layer + inference-protocol + versioning + agent-interface |
| 1 — Minimal Rust kernel | reader, typed schema, selection, histograms | **Done (one debt)** — `nano-rootio` (read **+ write**, local **+ remote**), `nano-core`, `nano-io`, `nano-producers` (muon), `nano-analysis` (`Hist1D`, typestate). Debt: golden test vs the frozen `.root` references not wired yet |
| 2 — Corrections & systematics | correctionlib, typed SF, weights, variations | **Mostly done** — native `nano-corrections` evaluator + typed SF + units + exhaustive `Systematic`; **JME weights + all five shape variations wired into the channel from the real `jet_jerc` payloads**, carried by `Weighted<R>`. Remaining: full four-vector JES/JER propagation (current slice is bookkeeping-weight scaffolding) + PU/muon-SF payloads |
| 3 — Semantic compiler | spec → semantic IR → validate → Rust codegen | **Core done** — `nano-spec` validate + derive `read_branches` + codegen, proven equal to hand-written (`nano-gen-demo`), incl. **inference codegen** (`nano-gen-tagger-demo`) |
| 4 — Workflow DAG (executor-agnostic) | typed workflow **DAG** as a portable IR: chunking, merging, provenance, staleness | **In progress** — `nano-workflow` typed DAG + local serial/parallel executor + portable JSON export + standalone task unit built; **thin Dask/Ray adapters built** (`integrations/`, not CI-gated). No built-in scheduler (LAW/HTCondor descoped) — the DAG is delegated to Dask/Ray/any system. *(Note: per the scope review, this is enabling infra, not the central thesis.)* |
| 5 — Agentic integration | agent-operable harness, semantic-diff review, validation/repair | **Started** — `nano-cli` + `nano-mcp` expose the compiler-gated action space; review/repair loops are future |
| 7 — UI / visualization layer | optional human cockpit over the DAG: capability-gated **web dashboard (kuva SVG) + ROOT browser** if kuva is present, else a **TUI** | **Planned** — the human counterpart to the MCP agent view; both front-ends share one UI-agnostic session core. See [`ui-layer.md`](ui-layer.md) |

Beyond the original plan, an **inference protocol** (`nano-inference`: mock / in-process
ONNX / remote / self-launching server, declared as `[[model]]`) was added as a boundary
layer.

**Foundations (Phases 0–1) and the core of the semantic compiler (Phase 3) are in
place; the next frontier is the Rust-native orchestrator (Phase 4).**

## Design decisions & refinements (stances taken for this project)

- **ROOT as boundary, already realized.** `crates/root-io` reads and writes ROOT
  TTrees; the rewrite keeps ROOT concepts out of the semantic core. RNTuple is a known
  gap (no read/write yet) — revisit if CMS NanoAOD migrates.
- **Golden tests already exist.** The C++ framework's frozen references in
  `tests/data/muon_validation/references/*.root` become the Phase-1 golden tests for the
  Rust kernel. Validation continuity is free — reuse, don't reinvent.
- **Typestate, used judiciously.** The `RawEvent → BaselineEvent → SignalRegionEvent`
  pattern is elegant for hand-written code but can fight ergonomics and codegen. Prefer
  enforcing stage/region invariants in the **semantic IR + generated code**, reserving
  hand-written typestate for a few high-value guardrails (e.g. "histogram fill requires a
  weighted, selected event"). Don't make every physicist touch phantom types.
- **Typed corrections; native evaluator is tractable.** Start with a typed Rust wrapper
  over correctionlib (FFI), but the correctionlib JSON schema is well-defined enough that
  a **native Rust evaluator** is realistic and removes a C++ dependency — aligns with the
  pure-Rust thesis. Either way, expose typed inputs (`MuonIdInput { pt, eta, year,
  variation }`), never `evaluate(vec![pt, eta, "nominal"])`.
- **Workflow DAG as a portable IR; delegate execution; LAW descoped.** The workflow
  layer is a **Rust-native typed DAG** — but we do **not** build our own distributed
  scheduler, and there is no LAW/HTCondor backend baked in. The DAG is a backend-independent
  **IR** (with a portable JSON export and a standalone Rust task unit per node); execution
  is **delegated to modern systems the user already runs — Dask, Ray, or any other** (the
  graph is JSON + a CLI atom, so Airflow/Snakemake/k8s/`make` work too). Rationale: the
  correctness/parallelism result ("if it compiles, it's safe to parallelize") makes the
  Rust per-chunk kernel a sound, self-contained atom; once it is, *who* schedules it is a
  swappable backend, and reusing Dask/Ray beats reimplementing a scheduler. The guarantees
  (order, staleness, provenance, sound parallel schedule) live in the IR + atom, so they
  hold under every backend. (See `docs/orchestrator.md`; the C++ Condor builders remain a
  reference for chunk/merge semantics.)
- **Where the thesis pays off fastest:** typed kernel + typed corrections + the
  validation/golden-test layer. That trio delivers "agent writes, compiler+validation
  reject mistakes" without needing the full semantic compiler. Prove it on the muon
  channel before investing in ADL→Rust codegen.

## Honest risk stance

- Rust HEP ecosystem immaturity → compatibility layers at the boundary; ROOT as I/O only.
- Over-engineering the type system → keep the *physicist-facing* layer simple YAML/ADL;
  confine advanced Rust to internal/generated code.
- False confidence from compilation → Rust catches *implementation* errors, not physics;
  golden/closure tests and physics validation reports remain mandatory.
- Rebuilding too much infra → keep the workflow IR clean with a thin batch/HTCondor
  *submission* target under the Rust-native orchestrator; don't reimplement storage/batch
  systems, only the typed DAG and provenance/staleness on top of them.
- Adoption → physicists edit specs and read reports, not Rust; keep ROOT-histogram /
  Combine-datacard outputs familiar.

## Pointer

`docs/rust-migration.md` is the concrete Phase-0/1 execution plan (I/O strategy, root-io,
uproot-as-oracle, staged kernel port). This doc is the umbrella it sits under.
