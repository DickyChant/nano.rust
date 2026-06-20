# Analysis state-machine model (design)

The implementation plan for the typed analysis state machine argued for in
`notes/main.tex` and `docs/vision.md`. Goal: encode the *event life cycle* in
Rust types so the compiler rejects out-of-order or incomplete analyses, while
keeping the ergonomic dynamic event access we already have.

## Two layers, deliberately separated

1. **Dynamic data access (exists today, `nano-core`).** `Event` /
   `Collection` / `ObjectView` give runtime-typed branch access
   (`obj.get::<f32>("pt")`, the `Prefix_attr` grouping). This stays — it is how
   you read arbitrary NanoAOD branches without code generation.
2. **Compile-time state machine (new).** A thin typed wrapper *over* a dynamic
   `Event` that tracks the analysis stage in the type, so transitions and
   preconditions are compiler-checked. The wrapper borrows the event; it adds
   no per-event allocation.

The point of the split: branch access is open and dynamic; analysis *structure*
(stage order, weighting, region, systematic completeness) is closed and static.

## The states

```
Raw ──preselect──▶ Baseline ──select(region)──▶ InRegion<R> ──weight──▶ Weighted<R>
                                  │
                                  └── veto ──▶ (event dropped; no token produced)
```

```rust
// Zero-sized stage markers.
struct Raw; struct Baseline;
trait Region { const NAME: &'static str; }
struct SignalRegion; impl Region for SignalRegion { const NAME: &str = "signal"; }

// Typed wrapper over a borrowed dynamic event; `S` is the stage.
struct Ev<'e, S> { inner: &'e nano_core::Event, _s: PhantomData<S> }

impl<'e> Ev<'e, Raw> {
    fn preselect(self, f: impl Fn(&Event)->bool) -> Option<Ev<'e, Baseline>>;
}
impl<'e> Ev<'e, Baseline> {
    fn select<R: Region>(self, f: impl Fn(&Event)->bool) -> Option<Ev<'e, R>>;
}
impl<'e, R: Region> Ev<'e, R> {
    fn weight(self, w: EventWeight) -> Weighted<'e, R>;
}
struct Weighted<'e, R> { ev: Ev<'e, R>, w: EventWeight }
```

`select`/`preselect` return `Option`: a vetoed event yields `None`, so the
*only* way to obtain an `Ev<SignalRegion>` is to pass the selection. Histogram
filling then *requires* the right token:

```rust
// Cannot be called with a Raw, unweighted, or wrong-region event:
fn fill<R: Region>(h: &mut Hist, e: &Weighted<R>, value: f64);
```

## Quantities, weights, systematics

- **Units**: newtype wrappers — energy `GeV`, cross-section `Fb`/`Pb`,
  integrated luminosity `FbInv`/`PbInv` (fb⁻¹, an *inverse* cross-section so it
  is a distinct type). Mixing requires explicit conversion; the one legal
  cross-section × luminosity product typechecks to a dimensionless event yield.
- **Weights**: `EventWeight` accumulates typed factors (pileup, SF, ...). A
  `Weighted<R>` is the proof that weighting happened before filling.
- **Systematics**: an exhaustive `enum Systematic`; the event loop is
  parameterized by it, so adding a variation forces every consumer to handle it
  (compile error otherwise). Shape vs. normalization carried in the type.

## How `nano-spec` (semantic IR) drives this

The hand-written wrappers above are the *target*. The semantic IR
(`docs/semantic-layer.md`, Slice A) is the *source*: a `muon.yaml` spec lowers to
a validated `AnalysisSpec`, from which we (a) derive the `read_branches` schema
for the streaming reader, and (b) generate the per-region selection/weight calls
expressed in these typed transitions. The muon codegen slice now emits this
typestate program directly and `nano-workflow` can run it as the scheduled
kernel; the hand-written `MuonProducer` is the golden reference for behavioral
equivalence.

What the compiler enforces (hard): stage order, region typing, weight-before-
fill, unit consistency, exhaustive systematics. What stays human/tested (soft):
whether the *cuts themselves* are the right physics (golden/closure tests).

## First implementable slice

1. Add the stage markers + `Ev<S>` / `Weighted<R>` wrappers in a new
   `nano-analysis` crate (or a `nano_core::sm` module), borrowing `Event`.
2. Re-express the existing muon selection using the wrappers (compile-checked),
   keeping behavior identical (validated by the existing skim tests).
3. Add a `fill`-requires-`Weighted<R>` histogram stub to demonstrate the
   precondition is a compile error when violated.
4. Only then: `nano-spec` IR + (later) codegen into these transitions.

Deliberately deferred: full histogram/datacard machinery, codegen, workflow IR.
Keep the typestate a scalpel (a few load-bearing guarantees), not a hammer —
physicists edit YAML/producers, not phantom types.
