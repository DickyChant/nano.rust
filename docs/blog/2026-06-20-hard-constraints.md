# Hard constraints for agentic physics analysis

*2026-06-20 — a working draft; this note will grow into an arXiv submission.*

Agentic coding tools can now write, refactor, and extend high-energy-physics
(HEP) analysis software faster than anyone can review it. The bottleneck is no
longer *writing* code — it is *guaranteeing* that the written code is correct.
This note argues that the way we currently steer agents cannot provide that
guarantee, and that the fix is to move correctness out of soft guidance and into
a compiler.

## Soft constraints have a ceiling

System prompts, agent "skills," execution harnesses, and human review all share
one property: they shape the *distribution* of what an agent produces without
constraining its *support*. A prompt is advisory. A passing test suite is
evidence, not proof — it cannot cover every region × process × systematic. Human
review is the traditional gate, but it scales poorly against machine-speed code
churn, which is exactly the effort agents are meant to save.

None of these can mechanically exclude the bugs that matter most, because those
bugs are not crashes — they are plausible-looking code that returns a subtly
wrong number:

- reading a branch that doesn't exist for a given data era;
- filling a signal-region histogram *before* the selection is applied;
- propagating a jet-energy-scale shape variation but forgetting its normalization;
- adding a quantity in GeV to one in MeV;
- reusing a cached output after the selection changed.

Each is an *implementation* error with *physics* consequences, and each is
invisible to a prompt and easy to miss in review.

## A hard constraint is a typed state machine

To raise the ceiling we need a mechanism that makes bad states impossible to
express, not merely discouraged. So model the analysis as a typed object, and in
particular model the **life cycle of an event** as a state machine whose
transitions are the only way to move between states. Encode it in the type
system, and "don't do X" becomes a compile error.

```rust
struct Raw; struct Baseline;
trait Region { const NAME: &'static str; }
struct SignalRegion; impl Region for SignalRegion { const NAME: &str = "signal"; }

struct Ev<'e, S> { inner: &'e Event, _s: PhantomData<S> }
struct Weighted<'e, R> { ev: Ev<'e, R>, w: EventWeight }

// Cannot be called with a Raw, unweighted, or wrong-region event:
fn fill<R: Region>(h: &mut Hist, e: &Weighted<R>, value: f64);
```

The only way to obtain an `Ev<SignalRegion>` is to pass the selection; the only
way to get a `Weighted<R>` is to weight it; and `fill` demands both. Units
become newtypes — energy in `GeV`, *cross-section* in `Fb`/`Pb`, *integrated
luminosity* in `FbInv`/`PbInv` (fb⁻¹) — so confusing a femtobarn with an inverse
femtobarn, or adding a cross-section to a luminosity, fails to compile; the one
legal product, σ × L, typechecks to a dimensionless event count. Systematics
become an exhaustive `enum`, so adding a variation makes incomplete code fail to
build. Above this sits a physics-facing spec that lowers to a typed IR and is
*statically validated* — every branch exists with the right type for the era,
every correction is available, every region is orthogonal — before any event
loop runs.

A compiler can't decide whether a 30 GeV threshold is the right *physics*. It
*can* decide that the code implementing it is type-correct, unit-correct,
stage-correct, and complete over its variations — exactly the errors that today
rely on review and luck.

## Why Rust

The argument needs a language whose type system can carry the state machine and
whose compiler is a hard gate. Rust qualifies — ownership and lifetimes make the
borrowed per-event views explicit and checked; `Option`/`Result` and exhaustive
`match` replace silent nulls. And it brings four systems strengths HEP needs:
compiled **performance**; **FFI** to trusted legacy libraries (ROOT,
correctionlib, combine, pyhf) at the boundary; **SIMD** per-event execution; and
a rich **TUI** ecosystem useful both for human orchestration tooling and for
agent-driven execution.

## Correctness is (most of) the parallelism proof

The deepest payoff is a near-tautology in Rust: **if it compiles, it's safe to
parallelize.** A per-event kernel that satisfies the spec is one the borrow
checker has already proven to have no hidden aliasing and no shared mutable state
escaping its declared inputs — which is *exactly* the soundness condition for
parallel execution. It's not a second proof; it's the same proof.

So the **schedule becomes free.** The analysis is really a dependency graph
(objects ← branches, cuts ← objects, weights/outputs ← objects + corrections,
systematics as a fan-out); the typed state machine is just one *linearization* of
it — a serial event loop. Any schedule that respects the dependencies is equally
legal, so the *same* verified kernel runs event-at-a-time for clarity,
column-at-a-time for SIMD, or chunk-parallel across cores — each correct by
construction. You verify *what* once; the framework picks *how*.

And the compiler **guides** it: a `Fn(&Event) -> Row` over `Send` data
parallelizes through `rayon`'s `par_iter` by construction; share state through a
non-thread-safe reference and it simply won't compile until you make it
thread-safe — the compiler rejects the unsafe schedule instead of letting a
silent data race through. Reductions (histograms) are a safe parallel-reduce.
This is also the real performance lever: correctness-preserving parallelism is
essentially *free*, where C++ makes you audit for races and Python can't express
it safely at all.

## It's buildable today

[nano.rust](https://github.com/DickyChant/nano.rust) is the realization in
progress. The foundation already works, in pure Rust: it reads and writes ROOT
`TTree`s, reads real CMS NanoAODv9 both locally and **remotely on demand** over
HTTPS byte-range (the first ten events of a 2 GB file fetch ~1.3 MB), streams a
real skim in ~72 MB of memory regardless of file size, and is **value-validated
against uproot** in CI on every push. The semantic IR and the typed state
machine are the next layers.

The takeaway: as code authorship is delegated to agents, correctness has to
migrate from soft guidance into hard, compiler-enforced structure. The faster
agents write, the more the compiler must be the thing that says *no*.
