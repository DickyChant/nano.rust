# Preservation & evidence: why a validated spec stays correct through to the kernel

This is the write-up the roadmap (`docs/compiler-roadmap.md` #8/#9) calls for: the
argument that nano.rust's compiler **preserves meaning** from a physicist-reviewed
spec down to the executed kernel, and the concrete evidence that backs it. It is
deliberately honest about what is proven vs. what is still scaffolding.

## The pipeline and where meaning is decided

```
ADL / TOML surface  →  AnalysisSpec  →  Core IR  →  KIR  →  { interpret | codegen → rustc }
   (front-ends)         (typed)        (typed +     (single        (two back-ends, one
                                        effects)     executable      semantics)
                                                     semantics)
```

- **Front-ends** (`from_adl_str` / `from_toml_str`) are *surface syntax only*. They
  desugar into the **same** `AnalysisSpec`; nothing else reaches execution. Proven:
  ADL and the equivalent TOML produce equal `AnalysisSpec` / Core IR / `ResolvedPlan`
  and identical execution + an identical preservation certificate (hash included).
- **`validate(spec, catalogue) → ResolvedPlan`** is the only place physics *meaning*
  is decided. It lowers to the typed **Core IR** (`nano-spec::core`: an `ExprNode`
  arena with a `Type`/`Effect` lattice and a primitive registry) and derives the read
  schema from the Core IR's `ReadsBranch` effects.
- **KIR** (`nano-spec::kir`) is the **single executable semantics**: the interpreter
  *executes* KIR and codegen *emits* from KIR. Drift between the two is eliminated by
  construction, then checked empirically (see fuzzing below).

## The two verifiers that compose

Correctness is enforced by **two** verifiers, each owning what it can actually know:

1. **The front-end validator** (`nano-spec::validate`) checks *domain facts* rustc
   cannot: the branch exists for the era with the right type, units are present and
   consistent, objects/regions/derived objects are defined, a model output is produced
   before use, output names are unique.
2. **rustc**, via the codegen target being the `nano-analysis` **typestate**
   (`Raw → Baseline → Scored<M> → Region → Weighted<R,S> → fill`), checks the
   *structure* of the generated kernel: stage order, region typing,
   score-before-use, weight-before-fill, and **exhaustive systematics** (a closed
   `Systematic` axis + `SystematicVisitor` — a missing variation arm is a compile
   error). A kernel with no shared mutable state is safe to parallelize.

Do **not** conflate them: branch existence is the *validator's* job, not rustc's. For
the compiled/JIT back-ends the second verifier *is the compiler that also produces the
executable*, which is why "if it compiles, it is safe to parallelize" is a property of
the executor.

## The preservation certificate

`PlanCertificate` (`nano-spec::certificate`, `nano certify <spec>`, and an MCP action)
is the machine-checkable face of the contract: a deterministic, serializable summary
of a validated plan — required branches (+types), outputs, histograms, the systematic
axis, shape corrections, weight systematics, model outputs, the Core IR effect set —
plus a fixed-seed FNV-1a **content hash** over a canonical serialization. It depends on
*meaning*, not surface syntax (ADL and TOML yield byte-identical certificates), and its
`required_branches` provably equal both the plan's derived read schema and the Core IR
`ReadsBranch` effects — so the certificate cannot silently drift from what the analysis
actually reads. This is the diffable artifact a semantic-review / agentic layer keys on.

## The evidence (what is actually proven, with the tests that prove it)

**Bit-identity (table stakes, not the differentiator).** Every generated kernel is
proven equal to a hand-written reference *and* the interpreter over synthetic events
(the `nano-gen-*` crates), and the Higgs→4ℓ open-data counts match ROOT exactly
(4μ=9115, 4e=5528, 2e2μ=12065, total 26708; `NANO_RUN_HTTP_TESTS=1`). This proves I/O +
codegen are correct on existing examples. It does **not**, by itself, prove the harness
catches *agent* mistakes — that is what the next two layers are for.

**The harness rejects mistakes (adversarial matrix).** `crates/nano-spec/tests/
adversarial_reject.rs` — one positive + one rejected case per error class, each
attributed to the right verifier: nonexistent/mistyped branch, era/version mismatch,
dropped unit, wrong-type access, undefined object, score-before-inference, duplicate
output → **validator** (`SpecError`); fill-before-weight, missing systematic arm →
**rustc** (`compile_fail` doctests). Closing a found gap: duplicate output names were
not previously rejected; now they are.

**The two back-ends agree (differential fuzzing) — and it found real bugs.**
`crates/nano-gen-demo/tests/differential_fuzz.rs` — a seeded, dependency-free generator
emits 400 valid specs (flat + pair/nested/cross-collection derived objects,
multi-region, sum/leading region requirements, weight/shape systematics) and asserts
the KIR interpreter == the compiled generated producer. It caught **three genuine
interpret-vs-codegen divergences**, since fixed:
1. cut comparisons emitted in `f32` while the interpreter used `f64` (disagree at the
   threshold) → codegen now compares in `f64`;
2. ΔR computed as `f64::from(eta - eta)` (subtract in f32 then promote — catastrophic
   cancellation) → promote first;
3. a derived object with no valid combination was eagerly evaluated by the interpreter
   instead of skipping the fill / failing the region → fixed to match codegen.
This is the empirical core of the "if it compiles it's correct" claim: the harness
caught its own mistakes.

**The harness generalizes (two prose-authored non-Higgs analyses).** Each authored from
a prose description and proven against an *independent* imperative reference
(`generated == reference == interpreter`, plus ADL == TOML):
- **Z→μμ control region** (`nano-gen-zmumu-demo`) — fully expressible *as-is*, **zero
  compiler changes**.
- **Multijet HT** (`nano-gen-multijet-demo`) — maximally different (no leptons/pairs);
  it surfaced two real IR gaps in *region requirements* (dimensioned `sum(...)` and
  `leading(...).attr`), which were then **closed**, after which the full prose
  validates and still matches the reference.

## Honest limitations (what this does NOT yet prove)

- **Bit-identical-to-ROOT is validation, not the differentiator** — we reproduced
  ROOT's own examples; matching ROOT on ROOT's examples is table stakes. The
  differentiator is mechanical safety on *agent-authored* analyses.
- **No external blinded benchmark yet.** The generalization analyses use *independent*
  references, but written in this repo. A blinded benchmark against a fully external
  (non-self) ROOT implementation is still open (#8).
- **String codegen.** Codegen emits *from KIR* but is still string-based; the typed
  `syn`/`quote` emitter (and `trybuild` preservation tests) is deferred. The
  differential fuzzer substantially hardens the string path (it found the 3 bugs).
- **Scaffolded breadth.** Systematics currently map onto a fixed closed `Systematic`
  enum (per-analysis *generated* variants are future work); units are GeV/dimensionless
  (no full `Quantity<Dim>` lattice yet); corrections are pt-scale shapes (full
  correctionlib-payload JES/JER deferred); ONNX inference is a boundary, not wired.

## One-line claim

nano.rust is a compiler for analysis-description specs whose IR is typed and whose code
generator emits Rust, so a validated analysis is mechanically safe — two verifiers
compose (domain-fact validator + the typestate that rustc checks), one KIR semantics
drives both back-ends, a content-hashed certificate makes preservation diffable, and
the adversarial / differential-fuzz / generalization evidence shows the harness rejects
mistakes, keeps its back-ends in agreement (catching real bugs), and extends to new
analyses.
