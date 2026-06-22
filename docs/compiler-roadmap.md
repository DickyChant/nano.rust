# Compiler roadmap — making the *general* claim real

We are **not** narrowing scope to a constrained subset. Instead we turn the gaps
a scope review flagged into an engineering plan: make `nano-spec` a real
multi-stage compiler so the broad claim — *a mechanically-safe compiler for
general HEP analyses* — becomes true and provable. (Source: Codex ideation,
2026-06-21.)

## The core move: a real compiler pipeline

Today `nano-spec` codegen is a string-emitting feature matrix that hand-branches
over expression cases, and model-aware codegen rejects derived objects. Replace
that with:

```
Surface AST  →  Typed Core IR  →  Kernel IR (KIR)  →  Rust AST (syn/quote)
  (ADL/TOML/YAML front-ends desugar into Core IR; nothing else reaches execution)
```

Core data structures: arena-backed `ExprNode` with `ExprId`/`ObjectId`/
`RegionId`/`VariationAxisId`/`ModelId`; `Type::{Bool,Int,Float,Quantity(Dim),
ObjectSet,Candidate,Event,Weight,Tensor,Histogram}`; `Effect::{ReadsBranch,
RequiresModel,ProducesScore,ShapeDependsOn(axis),RequiresCompat(mode)}`.

**Primitive registry** — `PrimitiveSpec { name, signature, dimension_rule,
effect_rule, lower_to_kir }`. New physics functions/operators are *registered*,
not hand-branched in codegen; ADL/TOML/YAML desugar onto the registry.

## Gap → solution (don't avoid the gaps; solve them)

1. **ADL at scale** — permissive `AdlAst` → desugar `object/region/define/comb/
   alias/bins/tables/weights/systematics` into typed Core IR. Only typed Core IR
   reaches execution. Proof: feature-coverage matrix, TOML/ADL equivalence,
   reject tests (bad ref/unit/alias-cycle/table-column).
2. **Typed lowering (KIR)** — SSA-ish `ValueId`, typed `Block`/`Stmt::{Let,ForEach,
   If,MatchVariation,Fill,Return}`, `Call(PrimitiveId)`; emit Rust via `syn`/`quote`.
   Every KIR instr typechecked before emission. Proof: KIR verifier + snapshot KIR.
3. **General typestate** — generated per-analysis `Region` markers; a generated
   closed `Systematic` axis + `SystematicVisitor` (one method/variation, so adding
   one breaks incomplete consumers); `Weighted<R,S>` (region × variation); `fill`
   requires `Weighted<R,S>` and `Hist<R,S,D>`; units become `Quantity<Dim>` over a
   dimension-vector lattice. Proof: `trybuild` compile-fail per invariant.
4. **Correction propagation** — `CorrectionNode { payload, inputs, output:
   NormFactor<Axis> | ShapeTransform<Collection,Axis> }`. JES/JER =
   `Jet@Nominal → Jet@JesUp/...`, so dependent selections/observables recompute per
   variation. Proof: threshold-migration + correctionlib reference-point tests.
5. **Inference + combinatorics** — `InferenceNode` over batch scopes
   `Event|Collection|CandidateSet`; features are typed exprs over the scope (incl.
   derived masses); scheduler inserts inference before dependent cuts. Real
   `InProcessPredictor` via `ort` (`onnx`). Proof: derived-candidate tagger,
   mock-vs-ONNX fixture, shape/dtype reject.
6. **ROOT-compat as opt-in** — drop `nearest_mass_truncated` as a normal primitive;
   put it behind `NumericCompatMode::RootDf103` carrying `RequiresCompat(mode)`,
   rejected unless enabled. Canonical primitive is `nearest_mass`. Proof:
   canonical-vs-compat goldens + reject-when-omitted.
7. **Generalization evidence** — adversarial suite (positive + reject/compile_fail
   per error class); blinded benchmark (prose note → agent spec → compile → compare
   to an independent ROOT impl); ≥2 non-Higgs/non-muon analyses (a JME/MET-heavy
   one; an ML/fatjet or ℓ+jets one with ONNX + b-tag/JES variations).
8. **No interpreter/codegen drift** — both execute **KIR** (interpreter becomes a
   KIR interpreter); property/differential fuzzing interpret == compiled == JIT.
9. **Two-verifier preservation** — contract: `validate(surface,catalogue)→CoreIR`
   (domain validity); `lower(CoreIR)→KIR` (preserves types/effects/deps);
   `emit(KIR)→Rust` (preserves typestate obligations); `rustc` (structural). A pass
   *certificate* (IR hashes, required branches, units, axes, model outputs,
   effects). Proof: KIR verifier + compile-fail suite + differential fuzzing +
   per-primitive translation tests.

## Staged roadmap (rough) — status as of the 2026-06-21→22 build

1. **DONE** — Core IR + primitive registry; lower TOML/YAML into it (`nano-spec::core`).
   Dimension lattice partial (sum/attr carry dimensions; full `Quantity<Dim>` deferred).
2. **DONE** — KIR + verifier; the interpreter **executes** KIR and codegen **emits**
   from KIR (`nano-spec::kir`). Both back-ends share one semantics → no drift.
3. **PARTIAL** — codegen emits *from KIR* (structural goal met). Still string-based;
   the syn/quote typed emitter + `trybuild` preservation tests remain deferred polish.
4. **DONE** — `Weighted<R,S>` + per-analysis generated closed `Systematic`
   enums and exhaustive `SystematicVisitor` (compile_fail proof); weight +
   shape systematic histogram fan-out **executes** (interpret==codegen) with
   declaration-derived variant names. Remaining: table fan-out.
5. **DONE (spec-declarable, payloads are content)** — corrections wired into the spec via
   the native correctionlib-v2 evaluator: `[[correction]]` kind="scale_factor" (SF →
   weight) and kind="jes" (binned correctionlib JES → kinematics, recomputes dependent
   selections per variation), evaluated by the SAME evaluator in interpret + codegen. Plus
   the **output→statistics handoff**: ROOT `TH1F` shapes, a **multi-process Combine
   datacard**, a **sample table** with per-sample xsec·lumi/sumw normalization, and a full
   worked example (samples → datacard). Remaining is real CMS payload/dataset CONTENT, not
   framework capability.
6. **DEFERRED** — inference over scopes / ONNX provider (a `nano-inference` boundary
   exists; real `ort` not wired — deferred from the thesis narrative per scope review).
7. **DONE** — ADL front-end: `from_adl_str` desugars to the SAME AnalysisSpec/Core IR/
   ResolvedPlan as TOML (proven equal + execution-equal); covers objects/regions/
   define/alias/outputs/histograms/weight-systematics/shape-corrections.
8. **PARTIAL** — adversarial reject matrix (9 classes) **DONE**; differential fuzzing
   (400 cases, found+fixed 3 real interpret-vs-codegen bugs) **DONE**; **two** non-Higgs
   analyses authored from prose (Z→μμ, multijet HT), each == an independent imperative
   reference **DONE** (the multijet one found 2 region-requirement IR gaps, since
   closed). Remaining: a blinded benchmark vs an *external* (non-self) ROOT oracle, and
   the preservation write-up.

The two-verifier preservation contract is real and tested: `validate` (domain facts:
branch/era/units/regions/output-before-use) + the typestate/`rustc` (stage/region/
weight-before-fill/score-before-use/exhaustive-systematic) — proven by the adversarial
matrix (validator rejects) and the nano-analysis compile_fail doctests (rustc rejects),
with differential fuzzing showing interpret==compiled across the supported surface.

## Highest-leverage moves & risks

**Moves:** (1) Core IR + registry first (stops sprawl); (2) KIR as the single
executable semantics (kills drift); (3) generalize variation axes before more
correction payloads; (4) turn evidence into CI (`trybuild`, differential fuzzing,
blinded reports).

**Risks:** ADL/CutLang edge cases (track coverage; lower only through Core IR);
Rust type noise (hide in generated crates; keep specs simple); correction-payload
diversity (conformance tests before broad claims); ROOT numeric parity
contaminating semantics (keep compat explicit/opt-in).

Each step keeps the existing specs **bit-identical** (the demos are the
regression gate) — we refactor the engine behind a stable facade.
