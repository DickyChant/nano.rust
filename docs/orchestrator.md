# Rust-native orchestration: the typed workflow DAG (design)

Phase 4. The LAW backend is **descoped** (`docs/vision.md`); the workflow layer
is in-language. This is the design the first `nano-workflow` slice implements.

## Why this layer exists

The typed kernel (`nano-analysis`) makes a *single event loop* correct. A real
analysis is many event loops over many files, with merges, systematic fan-out,
and outputs that must not go stale when an input changes. Those are
*workflow*-level invariants, and today they live in shell scripts and Condor
DAGs where they are unchecked. We lift them into a typed graph so the same
"make invalid states unrepresentable" discipline applies to the *schedule*, not
just the event.

What the workflow layer must guarantee (the error classes it removes):

- a **merge runs after** the maps it consumes — never before, never on a partial set;
- a **stale output is never silently reused** — if an input file, the spec, or
  the kernel changed, the artifact depending on it is recomputed;
- **provenance is recorded** — every artifact knows the inputs + code/spec
  version that produced it;
- **the schedule is sound by construction** — any order respecting the edges is
  legal, so serial and parallel runs are the same computation (this is the
  `nano-analysis` parallelism result, lifted to the graph).

## The graph

A workflow is a DAG of typed nodes; edges carry typed artifacts.

```mermaid
flowchart LR
  subgraph Source
    f1["input file / URL #1"] --> c1["chunk nodes<br/>(entry ranges)"]
    f2["input file / URL #2"] --> c2["chunk nodes"]
  end
  c1 --> m1["map: run kernel on chunk<br/>-> PartialOutput"]
  c2 --> m2["map: run kernel on chunk"]
  m1 --> r["reduce: merge partials<br/>(skim rows ++, hists +)"]
  m2 --> r
  r --> s["sink: write skim + merged hists<br/>+ provenance manifest"]
```

- **Source / chunk** — each input (local path or HTTPS URL) is split into
  bounded entry ranges via the existing `events_chunked` / `events_url_chunked`,
  so memory is bounded regardless of dataset size. One chunk → one map node.
- **Map** — runs the per-event kernel over one chunk, yielding a
  `PartialOutput` (skim rows + partial `Hist1D`s + a cutflow). The muon slice now
  exercises the closed path: `nano-spec` emits a `nano-analysis` typestate
  kernel, and `nano-workflow` runs that generated kernel through the same map
  node shape as the hand-written reference. Map nodes are **independent** — the
  fan-out point.
- **Reduce** — merges partials associatively: rows concatenate, histograms and
  cutflows sum (a parallel-reduce, per `nano-analysis`). Systematics are a
  fan-out dimension here: one reduce per `Systematic`.
- **Sink** — writes the merged skim (`nano_io::write_events`) and histograms,
  plus a **provenance manifest**.

## Typed node states (the workflow typestate)

Each node moves through a small state machine, mirroring the event-level one:

```
Pending ──inputs ready──▶ Ready ──run──▶ Done(artifact)
   │                                          ▲
   └────────────── Stale ◀────────────────────┘  (input/param hash changed)
```

A `Reduce` node is simply *unconstructable* until its `Map` dependencies are
`Done` — the dependency is a typed value it consumes, so "merge before map" does
not compile, the same trick as `fill` requiring `Weighted<R>`.

## Provenance & staleness

Every artifact carries a **key** = hash of:

1. its input artifact keys (or, for sources, the file's content/size + a chunk
   descriptor),
2. the spec / `read_branches` it was produced under,
3. a kernel/code version stamp.

A node is **stale** if its recomputed key differs from the key recorded in the
manifest next to its output. Up-to-date nodes are **skipped** — re-running a
workflow after changing one file recomputes only the affected chunks and the
merges above them. Staleness is thus a *detectable, typed* condition, not a
"did someone remember to delete the cache?" guess. The manifest is plain JSON
(reproducible, diffable, no DB).

## The executor (where the parallelism proof pays off)

The DAG is just nodes + dependency edges; **any topological order is legal**.
So one verified graph runs under multiple executors with identical results:

- **serial** — one thread, for clarity / debugging;
- **rayon-parallel** — independent map nodes across cores; because `Event` is
  `Send + Sync` and the kernel has no shared mutable state, this is safe *by
  construction* — the borrow checker already proved it (see the parallelism
  note). Reduce is a parallel-reduce.

Serial vs parallel producing bit-identical output is an **assertion the
executor checks**, not a hope — the same discipline as the `bench_parallel`
demo, lifted to the workflow.

## The DAG is the IR; executors are pluggable backends

We do **not** build our own distributed scheduler, and there is **no** batch
(HTCondor/SLURM/LAW) integration baked in. Instead, the typed DAG is a
**backend-independent intermediate representation**, and execution is delegated
to whatever modern system the user already runs — Dask, Ray, or anything else.
A clean DAG is intrinsically friendly to any executor; we just have to expose it
as one.

Two pure-Rust pieces make that work:

1. **A portable serialization** of the `WorkflowPlan` — JSON listing nodes,
   dependency edges, and each task's spec (input source + entry range +
   kernel/spec id). Any orchestrator can ingest this; it is the interchange
   format.
2. **A standalone task unit** — Rust entry points that execute *one* node
   without the orchestrator: `run-chunk` (read a chunk → run the kernel →
   write a serialized `PartialOutput`) and `merge` (reduce partials → merged
   output / skim). This is the atom an external scheduler invokes; the verified
   Rust kernel stays the unit of compute, so correctness/parallelism guarantees
   travel with it regardless of who schedules.

Backends, all running the *same* DAG to the *same* result:

- **Local** (`Executor`, built) — in-process serial or rayon-parallel, with the
  serial==parallel assertion.
- **Dask / Ray** (adapters) — a thin (~30-line) Python shim reads the portable
  graph and submits each `run-chunk`/`merge` unit as a `dask.delayed` /
  `ray.remote` task, inheriting their clusters, autoscaling, and dashboards. The
  Python is a *boundary adapter* (like uproot in tests), not a runtime
  dependency — the compute is the Rust atom it shells out to.
- **Anything else** — because the graph is JSON and the unit is a CLI
  invocation, Airflow, Snakemake, Nextflow, k8s Jobs, or a plain `Makefile` can
  drive it too. No backend is privileged; none is required.

The guarantees (node order, staleness, provenance, sound parallel schedule) live
in the IR and the Rust atom, so they hold under every backend — the scheduler
only decides *where/when* tasks run, never *what* they compute.

## How it connects to the rest

- **Input:** a validated `ResolvedPlan` (`nano-spec`) gives `read_branches` and
  codegen emits the per-event function as a `nano-analysis` typestate program; a
  dataset list gives the source files/URLs. `MuonProducer` remains the golden
  hand-written reference used by equivalence tests, not a separate scheduler
  path.
- **Output:** merged skim (`nano_io`) + histograms + the manifest. Eventually
  `nano run <spec> --inputs <list> [--systematics all]` builds and executes the
  DAG — the CLI/MCP "run" verb on top of the same compiler-gated action space.

## Slice 1 — typed DAG + local executor (`nano-workflow`) — **built**

Deliberately narrow, end-to-end, hermetic:

1. Artifacts: `ChunkSpec { source, entry_range }`, `PartialOutput { rows, hists,
   cutflow }`, `MergedOutput`.
2. A planner: inputs + `BranchSchema` + a kernel `Fn(&Event) -> Option<Row>`
   (+ optional hist fills) → source/map/reduce/sink nodes (chunks via
   `events_chunked`).
3. Executor with `serial` and `parallel` modes; **assert identical** merged
   output across the two (the proof, at workflow scale).
4. Provenance manifest (JSON) + staleness skip; **re-running is a no-op** when
   nothing changed, and touching one input recomputes only its sub-graph.
5. Sink writes the merged skim via `nano_io::write_events`.
6. Tests (hermetic): write a small ROOT file with `write_synthetic`, run the DAG
   over it serial vs parallel (identical), run twice (second run skips), and
   check the merged skim equals the single-pass `MuonProducer` result. The
   generated muon producer is also registered as a workflow kernel and its
   `MergedOutput` is asserted equal to the `MuonProducer` workflow output.

## Slice 2 — portable IR + standalone task unit + Dask/Ray adapters

Make the DAG executor-agnostic:

1. **Portable export** — `WorkflowPlan` → a versioned JSON `PortableGraph`
   (`serde`): nodes, dependency edges, and per-task specs (source, entry range,
   kernel/spec id, output path). Round-trips back so our `Executor` can run an
   imported graph too.
2. **Standalone task unit** — CLI/library entry points that run *one* node with
   no orchestrator: `run-chunk` (read chunk → kernel → write serialized
   `PartialOutput`) and `merge` (reduce partials → `MergedOutput`/skim). These
   are the atoms any scheduler invokes; they reuse the exact `nano-workflow`
   compute so results match the local executor bit-for-bit.
3. **Adapters** — a thin Python shim (`integrations/`) that reads `PortableGraph`
   and submits each unit as `dask.delayed` / `ray.remote` tasks. Not CI-gated on
   Dask/Ray being installed; a documented, runnable example. The compute is the
   Rust atom it shells out to.
4. Tests (hermetic, Rust): export → re-import → run → equals the in-memory
   plan's `MergedOutput`; `run-chunk` + `merge` composed by hand equals the
   single-pass result (proving the atoms are faithful). The Python adapter is
   demonstrated, not unit-tested in CI.

Deferred: systematic fan-out beyond one reduce-per-`Systematic`, datacards/plots,
a graphical DAG view, and any *built-in* distributed scheduler (we delegate to
Dask/Ray/etc. instead). Keep the typed graph small and load-bearing — the
guarantees (order, staleness, provenance, sound parallel schedule), not a general
workflow engine.
