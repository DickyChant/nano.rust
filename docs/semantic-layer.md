# Semantic Layer — First Slices (Phase 2 & 3)

Builds on the Phase-1 kernel (read/write/stream real NanoAOD). This is the start of
the semantics-first design in `docs/vision.md`. Two parallel slices, each a new crate,
each deliberately *thin* — prove the shape before generalizing.

> **Status (both slices built; this records the original design).** Slice A
> (`nano-spec`) now goes past validation all the way to **codegen** — it generates the
> per-event kernel as a `nano-analysis` typestate program (and an inference-bearing
> kernel for `[[model]]` specs), proven equal to the hand-written `MuonProducer`. Slice B
> (`nano-corrections`) is a working native evaluator with JME weights/variations wired
> into the channel. For how the IR is *executed* (interpret vs codegen+AOT vs future
> JIT) and the two-IR picture, see [`architecture.md`](architecture.md). The "deferred"
> notes below are historical — codegen is no longer deferred.

## Slice A — Semantic IR (the differentiator), crate `nano-spec`

Goal: a physics-facing spec compiles to a **typed, validated** analysis object that
*drives* the kernel — not a script. Start with the muon channel. *(Originally this slice
stopped at validation + planning; codegen has since been built — see the status note
above.)* The validation + planning chain:

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

~~Deferred to a later slice: generating the Rust kernel from the IR.~~ **Done:** codegen
now emits the kernel as a `nano-analysis` typestate program (and routes `[[model]]` specs
through `Ev::infer`), proven equal to the hand-written `MuonProducer` — which is now just
the golden reference. The IR can also be *interpreted* directly (no compile); see
[`architecture.md`](architecture.md).

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
each other. Slice A is the headline; Slice B proves the corrections boundary. Both have
since landed, and the layers they teed up are built too: codegen (→ typestate kernel),
JME systematics/weights into the channel, and the workflow DAG (`nano-workflow`). What
remains: full four-vector JES/JER propagation, PU/muon-SF payloads, histogram/datacard
machinery, and a future JIT back-end — per the roadmap in `docs/vision.md` and the
back-end model in `docs/architecture.md`.
