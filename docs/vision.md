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
Workflow orchestration IR        (backend-independent; targets LAW first, Rust-native later)
```

ROOT/correctionlib/LAW/Combine/pyhf are **boundary integrations**. ROOT is a *storage
format*, not the semantic core — no `TTree`/`TH1`/raw branch strings leak above the I/O
layer. The analysis *meaning* lives in the typed IR, independent of any backend.

## Where we are now (status against the roadmap)

| Phase | Scope | Status |
|---|---|---|
| 0 — Design study | IRs, ADL-like syntax, design docs | **In progress** — this doc + `docs/rust-migration.md` |
| 1 — Minimal Rust kernel | NanoAOD reader, typed schema, selection, cutflow, histograms | **In progress** — `crates/root-io` (read **+ write**), `crates/nano-io` (reader), `crates/nano-core` (event model). Benchmark = the existing **muon** channel. |
| 2 — Corrections & systematics | correctionlib, typed SF API, weights, shape variations | **Next** — port C++ helpers (JME, PU, top-pt); decide FFI vs native correctionlib |
| 3 — Semantic compiler | YAML/ADL → semantic IR → validate → Rust codegen | Future |
| 4 — Workflow IR + LAW backend | chunking, merging, datacards, plots | Future (C++ side already has Condor builders to learn from) |
| 5 — Rust-native orchestrator | typed DAG, targets, provenance, staleness | Future / optional |
| 6 — Agentic integration | semantic-diff reviews, validation suite, repair | Future |

**We are squarely in Phase 1**, which is the correct foundation: every higher layer
consumes the typed event kernel and the I/O boundary.

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
- **Defer the native orchestrator.** Phase 5 competes with mature tooling (LAW, batch
  systems). Highest value-per-effort is the **Workflow IR + a LAW backend** (Phase 4);
  treat the Rust-native engine as optional and later. Define the IR first so the backend
  is swappable.
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
- Rebuilding too much infra → LAW + existing batch/storage first; backend-independent IRs
  before native alternatives.
- Adoption → physicists edit specs and read reports, not Rust; keep ROOT-histogram /
  Combine-datacard outputs familiar.

## Pointer

`docs/rust-migration.md` is the concrete Phase-0/1 execution plan (I/O strategy, root-io,
uproot-as-oracle, staged kernel port). This doc is the umbrella it sits under.
