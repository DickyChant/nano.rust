# Semantic Layer — First Slices (Phase 2 & 3)

Builds on the Phase-1 kernel (read/write/stream real NanoAOD). This is the start of
the semantics-first design in `docs/vision.md`. Two parallel slices, each a new crate,
each deliberately *thin* — prove the shape before generalizing.

## Slice A — Semantic IR (the differentiator), crate `nano-spec`

Goal: a physics-facing spec compiles to a **typed, validated** analysis object that
*drives* the kernel — not a script. Start with the muon channel; **no Rust codegen yet**
(that's the hard, later part). The chain for this slice stops at validation + planning:

```text
muon.yaml  ──parse──▶  AnalysisSpec (typed IR)  ──validate──▶  ResolvedPlan
                                                                  │
                                          derives the exact read_branches set
                                          + the checked selection/region graph
```

### Input (physics-facing), e.g. `specs/muon.yaml`
```yaml
analysis: { name: muon_demo, year: Run2018 }
objects:
  good_muon:
    source: Muon
    cuts: [ "pt > 30 GeV", "abs(eta) < 2.4" ]
regions:
  signal:
    require: [ "count(good_muon) >= 1" ]
outputs:
  - { name: n_good_muon, expr: "count(good_muon)" }
  - { name: lead_muon_pt, expr: "leading(good_muon).pt" }
```

### Typed IR (Rust)
```rust
struct AnalysisSpec { name: String, year: Year, objects: Vec<ObjectDef>, regions: Vec<RegionDef>, outputs: Vec<OutputDef> }
struct ObjectDef { name: String, source: String /* "Muon" */, cuts: Vec<Cut> }
struct Cut { lhs: Expr, op: CmpOp, rhs: Quantity /* value + Unit */ }
enum Expr { Attr{ object: String, attr: String }, Abs(Box<Expr>), Count(String), LeadingAttr{ object: String, attr: String } }
```

### Static validation (the payoff — catches agent/spec mistakes before any event loop)
- Every referenced attribute (`Muon_pt`, `Muon_eta`, …) exists in the NanoAOD branch
  catalogue for the spec's `year`/version, with a compatible `BranchType`.
- Units are present and dimensionally consistent (`pt > 30 GeV`, not `pt > 30`).
- Regions reference only defined objects; output exprs are well-typed.
- **Derive `read_branches`** from the spec → feeds `nano_core::BranchSchema`. This makes
  the vision's "the spec decides which branches exist" literally true, and connects the
  IR straight into the streaming reader we just built.

Deferred to a later slice: generating the Rust kernel from the IR (for now the
hand-written `MuonProducer` stands in; the IR validates and plans, the producer executes).

## Slice B — Typed corrections, crate `nano-corrections`

Goal: typed correctionlib evaluation; **native Rust** (no C++ FFI — keeps the pure-Rust
thesis and avoids the dep we just shed). Thin: one correction + nominal + one variation.

```rust
let sf = corrections.muon_id.evaluate(MuonIdInput {
    pt: muon.pt()?, eta: muon.eta()?, year: Year::Run2018, variation: Variation::Nominal,
})?;                       // never evaluate(vec![pt, eta, "nominal"])
```

- Parse a correctionlib JSON (the `data/jme-derived/*.json` payloads are the format).
- Evaluate the bounded expression tree: `Binning` / `Category` / `Formula` nodes.
- Typed inputs per correction; `Variation` enum drives nominal/up/down.
- First target: one muon scale factor, validated against a couple of reference points.

## Sequencing

Both are **new crates** (no edits to the kernel crates), so they don't conflict with
each other. They start once the streaming-reader work is committed (to keep Codex builds
from racing on the shared tree). Slice A is the headline; Slice B proves the corrections
boundary. Codegen, systematics propagation, histograms, and the workflow IR come after
these two land — per the roadmap in `docs/vision.md`.
