# Rust Migration Plan

## Why Rust

The framework's contract is **"agents write, you review."** Agents produce most of the implementation code; the human reviews physics, not syntax. That makes the *language's safety floor* the most important property: the cheaper it is to catch agent mistakes at compile time, the less physics-review effort leaks into chasing memory/aliasing bugs.

Rust fits this better than C++:

- **Explicit object lifecycles.** Ownership and borrowing make the lifetime of every event/object/collection explicit and compiler-enforced. The event loop is full of borrowed views into per-event buffers (`Collection`, `ObjectView`, attached `extra<T>` values) — exactly the aliasing-prone pattern where C++ gives no guarantees and Rust does.
- **Stronger compiler.** No undefined behavior in safe code, exhaustive `match`, `Option`/`Result` instead of sentinel values and silent nulls, no implicit numeric coercions. Agent-authored code fails loudly at `cargo build` instead of producing a subtly wrong skim.
- **Reproducible toolchain.** `cargo` removes the LCG/CMake/ROOT-dictionary build friction that currently gates every build and Condor job.

The C++ implementation is preserved on the `cpp-snapshot` branch and remains the behavioral reference until the Rust port validates against the same frozen `.root` references.

## ROOT I/O in Rust — the central constraint

NanoAOD is a **TTree** format (confirmed: both `NanoReader` and `RootOutputFile` use `TTree`/`TTreeReader`, not RNTuple). So the I/O question splits cleanly:

### Reading — viable with `root-io`

[`root-io`](https://crates.io/crates/root-io) (crate v0.3.0, MPL-2.0, part of `alice-rs`):

- **Pure Rust** — no ROOT/C++ install required. This alone removes most of the current build pain.
- Reads the ROOT binary format and **iterates TTree branches/columns** (`tree_reader` module).
- Proven against real physics data (ALICE public datasets), cross-checked against the C++ implementation.

Limitations to design around:

- **Read-only.** No write path.
- **No RNTuple.** Fine for current NanoAOD (TTree); a risk if CMS NanoAOD migrates to RNTuple later.
- **Lightly maintained / small API.** Expect to read NanoAOD branch layouts ourselves and possibly patch/vendor the crate. Verify it handles the exact branch types we bind (jagged `vector<float>`, the integer width zoo, bools) before committing — prototype a reader against one validation file first.

### Writing — make `root-io` a true read+write library

A crate that only reads is `root-i`, not `root-io`. The end-state decision is to **engineer the write path too**, in pure Rust, rather than treat writing as permanently external.

Write is genuinely harder than read — read parses whatever bytes exist; write must produce bytes that ROOT and every downstream tool accept. The hard parts:

- **TStreamerInfo generation** — ROOT files are self-describing; the writer must emit schema records for each branch type.
- **TFile bookkeeping** — the free-space list, the StreamerInfo key, the key list, and patching the header seek pointers (`fSeekFree`/`fSeekInfo`) at close.
- **Basket layout + compression framing** — ROOT wraps zlib/lz4/zstd in its own block header.

Two facts make this tractable for us specifically:

- **We control the output schema.** We don't need a general ROOT writer — only one that emits our fixed skim: an `Events` tree of the types in `RootOutputFile::BranchStorage` (`bool, i32, u32, u64, float, vector<float>`) plus filtered `Runs`/`LuminosityBlocks`. A tiny, fixed subset of the format.
- **The read side already has the inverse machinery.** `root-io` already models `TKey`/`TStreamerInfo`/`TBasket`/`TBranch` and already handles ROOT's compression framing. A writer reuses those type models and the compression layer; the new work is serialization-direction code, not the whole format. So root-io is **vendored in-tree at `crates/root-io`** (from `cbourjau/alice-rs`, MPL-2.0 — headers/LICENSE retained) and extended there as our own crate, rather than rewritten from scratch. It is a private fork (no upstream-contribution goal): ALICE remote-fetch (`alice-open-data`/`reqwest`) was dropped, and the write path is added in place.
- **`uproot` is the spec *and* the test oracle.** uproot (pure Python, BSD-3 — permissive) already reads *and* writes TTrees including jagged arrays. Two roles:
  - *Spec / porting reference:* its `uproot.writing` code implements exactly the hard parts (TStreamerInfo generation, basket writing, TFile seek bookkeeping, compression framing, jagged arrays). Port tested algorithms instead of reverse-engineering the binary format.
  - *Differential test oracle:* because uproot needs no ROOT/C++ install, it gives a ROOT-free CI loop in both directions — our writer emits → uproot reads back and asserts; uproot writes a reference → our reader parses and asserts; and cross-check `root-io` reads against uproot reads of the same NanoAOD. This validates the pure-Rust path without the heavy ROOT toolchain in CI.
  - *Discipline:* uproot is a **build/test/reference dependency, not a runtime one** — the shipping path stays pure-Rust.

**Plan of record:** extend `root-io` into full read+write; ship the narrow writer (our fixed output types) first, generalize only as new channels need new types. Validate against uproot as oracle, then against the frozen `.root` references.

**Interim fallback only:** a ROOT-FFI (`cxx`/`bindgen`) output writer is acceptable as a *throwaway* to unblock the producer port if the native writer lags — but it reintroduces the ROOT/C++ build dependency we are shedding, so it is not the goal. (Parquet/Arrow output is out — CMS workflows expect ROOT.)

Reading can start immediately in parallel with writer development.

## Staged plan

Each stage validates against the existing frozen references (`tests/data/muon_validation/references/`) — the bar is matching the C++ output, not re-deriving physics.

1. **Scaffold** a `cargo` workspace (core / io / producers / app crates mirroring `include/nano/*`). Keep the C++ tree building in parallel during the transition.
2. **Input layer**: prototype `root-io` against one validation NanoAOD file; confirm all bound branch types round-trip. Build the schema/branch-grouping layer (the `Prefix_attr` → object/attribute rule) on top.
3. **Event model**: `Event` / `Collection` / `ObjectView` with dynamic attachments, idiomatic Rust (borrows over the per-event buffers; `Option`/`Result` over sentinels).
4. **Output writer**: extend `root-io` with a native pure-Rust write path (uproot as spec + differential oracle); emit `Events` + filtered `Runs`/`LuminosityBlocks`. ROOT-FFI writer only as a throwaway if this lags the producer port.
5. **Port the muon channel** (`HeavyFlavBaseProducer` + `HeavyFlavMuonSampleProducer`) and the helpers it needs (JME, PU, top-pt, gen matching), keeping selection order aligned with the C++/Python reference.
6. **Validate** the Rust skim against the references; only then retire the C++ path.

## Delegation

Substantial mechanical work — scaffolding the workspace, porting a producer, wiring branch catalogues — is suitable to hand to **Codex** (`codex:rescue` agent / Codex runtime). Keep physics-bearing logic under human review per the framework contract.

## Config & references carry over unchanged

`configs/` (run cards, branch catalogues, sample lists), the validation references, and the `docs/` design rules are language-agnostic and stay as the source of truth across the port.
