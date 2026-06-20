# Inference protocol (design)

How nano.rust calls **external tools** — first and foremost ML inference
(per-object taggers like ParticleNet/DeepJet, per-event classifiers, and
generative/LLM models) — without the framework *becoming* a model runtime.

The design rule is the one modern OCR/VLM tools use: **the framework speaks a
protocol, not a backend.** You hand it an endpoint and it uses it; you hand it
nothing and it *launches its own server* and tears it down when done. The typed
spec and state-machine layers sit on top of the protocol, so they are identical
whether the model runs in-process, in a server we spawned, or on a GPU farm
across the building.

## Three things kept separate

1. **Protocol** — the wire contract: *send a batch of named, typed tensors out;
   get a batch of named, typed tensors back.* Nothing else. This is a Rust trait
   plus a canonical request/response schema.
2. **Provider (transport)** — *who answers the protocol*: an in-process library,
   a server we launched, a remote endpoint, or a deterministic mock.
3. **Binding** — *which branches feed the model and which typed column the output
   becomes* — declared in the spec, validated by the compiler/validator, and fed
   into `read_branches` derivation.

Keeping these orthogonal is the whole point: a physicist edits the binding (spec
TOML); an agent never has to know the transport; the transport can change
(laptop → cluster) with zero change to the analysis.

## The protocol

```rust
/// The entire contract. Send tensors, get tensors.
pub trait Predictor: Send + Sync {
    fn predict(&self, req: &InferRequest) -> Result<InferResponse, InferError>;
    fn metadata(&self) -> ModelMeta;        // declared inputs/outputs, dtypes, shapes
}

pub struct Tensor { pub name: String, pub shape: Vec<usize>, pub data: TensorData }
pub enum TensorData { F32(Vec<f32>), F64(Vec<f64>), I64(Vec<i64>), /* … */ }

pub struct InferRequest  { pub model: String, pub inputs:  Vec<Tensor> }
pub struct InferResponse { pub model: String, pub outputs: Vec<Tensor> }
```

The tensor request/response is deliberately shaped to match the **Open Inference
Protocol (KServe v2)** — the de-facto standard that Triton, MLServer, TorchServe,
and friends already speak over HTTP/gRPC. We adopt it rather than invent a
bespoke wire format, so "remote provider" is just a thin client against servers
that already exist. For **generative/LLM** models the same `Predictor` is
implemented over the **OpenAI-compatible** HTTP API (what vLLM and SGLang serve),
so an LLM is just another provider.

`Predictor: Send + Sync` is load-bearing: it means a provider drops straight into
the parallel event loop (`rayon`) — see "Inference is a graph node" below.

## Providers

All implement the one trait; resolution picks one from config.

| Provider | When | Native dep? | Notes |
|---|---|---|---|
| `MockPredictor` | tests, dry-runs, codegen | none (pure Rust) | deterministic from input hash; **default** so CI is hermetic |
| `InProcess` (ONNX) | small taggers, no server wanted | ONNX Runtime via `ort` (feature `onnx`) | PyTorch→ONNX is the standard HEP export path; lowest latency |
| `Remote` | a server already runs | none beyond the `http` stack | KServe v2 (tensors) or OpenAI API (LLM) over HTTPS |
| `Managed` | you have a model, not a server | spawns the chosen server | **launch-your-own**: spawn → health-poll → talk via `Remote` → kill on `Drop` |

### `Managed` — bring your own server

This is the OCR-tool behavior. Given a model and a launch recipe (or a built-in
one for known backends), `Managed`:

1. picks a free port, spawns the server process
   (`vllm serve …` / `sglang.launch_server …` / `tritonserver …` /
   `mlserver start …`, or a recipe you supply),
2. polls its health endpoint until ready (with a timeout),
3. serves every `predict` call through a `Remote` client to `localhost:port`,
4. kills the child on `Drop` — lifecycle is RAII, no leaked GPU processes.

```rust
pub enum ProviderSpec {
    Mock,
    InProcess { onnx_path: PathBuf },
    Remote    { endpoint: Url, api: WireApi },          // WireApi::{KServeV2, OpenAI}
    Managed   { launch: LaunchRecipe, api: WireApi },   // spawn, then Remote
}
// Resolution: endpoint given -> Remote; model+launch given -> Managed;
//             onnx_path given -> InProcess; else -> Mock.
```

A built-in tiny mock HTTP server backs `Managed` in tests, so the spawn → health
→ round-trip → teardown path is exercised hermetically with **zero** external
dependencies.

## Binding it in the spec

Inference is declared, not coded. A `[[model]]` block names the model, the input
branches, the output column, and (optionally) the provider — defaulting to
`mock` so the spec validates and `read_branches` is derivable with no server
present.

```toml
[[model]]
name    = "top_tagger"
inputs  = ["FatJet_pt", "FatJet_eta", "FatJet_phi", "FatJet_mass", "FatJet_btagDeepB"]
output  = "FatJet_topscore"          # becomes a typed per-object column
batch   = "FatJet"                   # one inference row per FatJet (per-object)

[model.provider]                     # optional; omitted => mock
kind     = "managed"                 # mock | inproc | remote | managed
launch   = "triton"                  # built-in recipe, or an explicit command
model_repo = "models/particlenet"

[regions.signal]
require = ["count(good_fatjet) >= 1", "leading(good_fatjet).topscore > 0.8"]
#                                                          ^ the model output,
#                                                            usable like any attribute
```

What the validator checks **before any event loop or server launch**:

- every `inputs` branch exists with a numeric type for the era (same check as any
  read branch) — and so the inputs are folded into the derived `read_branches`;
- `output` is a fresh column name with a declared dtype — downstream cuts that
  reference it (`leading(good_fatjet).topscore`) type-check against it;
- `batch` names a real object/collection, fixing the request shape;
- the provider spec is well-formed (a `remote` endpoint parses; a `managed`
  recipe is known or fully specified). Reachability/health is a **runtime**
  check at start-up, reported as structured error — not a silent fallback.

So a wrong feature name, a score used before it is produced, or a model wired to
a branch that doesn't exist in this era are all **rejected statically**, exactly
like the rest of the semantic layer.

## Inference as a typed state-machine transition

In the typestate model (`docs/state-machine.md`), inference is a transition that
consumes a batch and attaches a typed output column — it cannot be skipped if a
cut depends on it, and its output cannot be read before it runs:

```rust
impl<'e> Ev<'e, Baseline> {
    /// Attach model outputs; the result type carries proof the score exists.
    fn infer<M: ModelTag>(self, p: &impl Predictor, feats: Features<M>) -> Ev<'e, Scored<M>>;
}
// A cut/region that reads `topscore` requires Ev<.., Scored<TopTagger>>:
// reading it on an un-inferred event is a compile error, not a 0.0.
```

The point mirrors weight-before-fill: **score-before-use** becomes a type
precondition, generated from the spec rather than trusted by review.

## Inference is a graph node (so it parallelizes for free)

Per the parallelism thesis (`docs/blog/2026-06-20-hard-constraints.md`),
inference is just another node in the dependency DAG: it consumes typed feature
columns and produces a typed output column. Because `Predictor: Send + Sync`, a
chunk of events runs its features through the model under `rayon` with the same
"if it compiles, it's safe to parallelize" guarantee as the rest of the kernel.
And inference is *naturally batched/columnar* — the framework gathers a chunk's
features into one `InferRequest`, which is also the shape a GPU server wants, so
the columnar schedule and the efficient-inference schedule are the same schedule.
A `Remote`/`Managed` provider additionally hides latency by batching the chunk
into one round-trip.

## Build order

1. `nano-inference` crate: the `Predictor` trait + `Tensor`/`InferRequest`/
   `InferResponse`, `MockPredictor`, batched feature extraction from
   `nano-core` collections, and a parallel (serial-vs-`rayon`) demo proving
   identical results. Pure Rust, no native deps, fully in CI.
2. `Remote` (KServe v2 + OpenAI) behind the existing `http` feature, with a
   built-in mock server and a hermetic spawn→health→round-trip→teardown test for
   `Managed`.
3. Spec `[[model]]` parsing/validation + `read_branches` extension in
   `nano-spec`; codegen attaches the output column.
4. `Scored<M>` typestate transition in `nano-analysis`.
5. `InProcess` ONNX via `ort` behind a `onnx` feature (optional, never required
   for CI).

Deliberately deferred: model versioning/caching policy, multi-model ensembles,
and warm-pool management for `Managed` providers — add when a real analysis
needs them, keep the protocol surface small.
