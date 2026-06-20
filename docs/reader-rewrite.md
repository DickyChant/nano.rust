# Owned ROOT I/O core — rewrite plan

We have evolved the vendored `root-io` (NanoAODv9 versions, basket access,
streaming, zstd, typed errors). It works: it reads real CMS NanoAOD,
value-correct vs uproot, with bounded streaming. This plan is about replacing the
*reading internals* with a purpose-built core we fully own — and unifying it with
the writer.

## Why now (the calculus changed)

- **We have an oracle.** Real NanoAOD + uproot cross-check + writer round-trip.
  A rewrite is *verifiable*, so the original "you'll rediscover edge cases"
  risk is caught by tests, not shipped.
- **We understand the format.** We extended TTree/TBranch streaming and versions.
- **The vendored split is blocking us.** The jagged-writer `fLeafCount` fix is
  hard because ROOT's object-reference machinery lives in opaque vendored
  `Context`/`MAP_OFFSET` code the *reader and writer don't share*. A unified core
  fixes this by construction.

## Scope

**In:** the NanoAOD subset — TFile/TKey, the key list + streamer info, the four
compressions (zlib/LZMA/lz4/zstd, already done — reuse), TTree/TBranch/TLeaf for
flat branches (scalars + jagged-by-counter), basket read (seek+decompress+slice),
and an **explicit** object-reference/streamer layer shared by read and write.
Synchronous, typed errors, no async/`block_on`.

**Out:** general ROOT object model, ALICE ESD, RNTuple, `code_gen`. We don't use
them.

## The unification payoff

Read and write share: `TKey`, basket framing, streamer-info records, and the
intra-buffer **object-reference** scheme. Once that scheme is explicit and shared,
the writer's jagged `fLeafCount` (a back-reference to the counter leaf) is a
direct use of the same primitive — closing the last I/O interop gap.

## Strangler strategy (non-negotiable)

1. Build the new core behind the **same `nano-io` API** (`events`, `events_url`,
   `read_events`, the writer), in parallel with vendored `root-io`.
2. **A/B validate**: every existing test must pass on the new core —
   - real NanoAODv9 read, value-equal to uproot (the http + local tests),
   - bounded streaming skim (~tens of MB),
   - writer round-trip + uproot reads our output (incl. jagged),
   - the ALICE/sample read tests can stay on vendored root-io or be dropped.
3. Only switch `nano-io` to the new core when it is green on all of the above.
4. Keep vendored `root-io` available until the new core is proven, then retire it.

## Phasing

- **P1 — read foundation:** TFile/TKey + key list + streamer info + decompression
  (reuse) → open a file, list trees, read scalar branches; A/B vs uproot.
- **P2 — branches + baskets:** TTree/TBranch/TLeaf, basket windowed reads, jagged
  via counter; bounded streaming; A/B vs uproot on real NanoAOD.
- **P3 — unify writer:** move the writer onto the shared primitives; the explicit
  object-reference closes jagged `fLeafCount` (uproot reads our jagged output).
- **P4 — switch + retire:** point `nano-io` at the new core; retire vendored
  `root-io`.

## Risk

It is a real investment and the current reader already works, so the bar is the
unification payoff (owned, understandable, writer-unblocking) — not novelty. The
oracle harness is what makes it safe; do not switch `nano-io` until A/B is green.
