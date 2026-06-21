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

## Staged roadmap (rough)

1. Core IR + primitive registry + dimension lattice; lower current TOML/YAML into it.
2. KIR verifier + KIR interpreter; port interpreter behavior to KIR.
3. Typed Rust emitter (syn/quote); remove string-branch codegen; `trybuild` preservation tests.
4. Generic systematics, `Weighted<R,S>`, histogram/table fan-out, visitor exhaustiveness.
5. Correction graph (JES/JER shape + norm weights).
6. Inference over object/derived scopes; ONNX provider.
7. ADL front-end coverage (objects/regions/define/comb/bins/tables/weights/systematics/aliases).
8. Adversarial suite + blinded benchmark + two external analyses + the preservation write-up.

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
