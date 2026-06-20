# Watch the compiler reject a data race — then the 8x that follows

*2026-06-21 — a screencast of the parallelism thesis: the same verified kernel,
one unsafe schedule the compiler refuses to build, and one safe schedule that
runs ~8x faster with bit-identical results.*

The [hard-constraints note](2026-06-20-hard-constraints.html) made a claim that
sounds too good: *if it compiles, it's safe to parallelize* — the borrow
checker's soundness condition is the same condition parallel execution needs, so
the schedule is "free." This screencast is that claim, live, on real CMS
NanoAODv9.

<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.css">
<div id="player" style="margin:1.5rem 0"></div>
<script src="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.min.js"></script>
<script>
  AsciinemaPlayer.create('../demo-parallel.cast', document.getElementById('player'), {
    cols: 100, rows: 32, idleTimeLimit: 2, theme: 'asciinema', poster: 'npt:0:3'
  });
</script>

*(No player? Raw cast: [`demo-parallel.cast`](../demo-parallel.cast) —
`asciinema play demo-parallel.cast`.)*

## One kernel, two schedules

The per-event analysis kernel is written once. The serial and parallel runners
differ by exactly one method call:

```rust
fn collect_serial(events: &[Event])   -> Vec<Contribution> { events.iter().map(analyze_event).collect() }
fn collect_parallel(events: &[Event]) -> Vec<Contribution> { events.par_iter().map(analyze_event).collect() }
```

`.iter()` → `.par_iter()`. That this even compiles is the point: `Event` is
`Send + Sync` (its columns are `Arc`-shared), so `rayon` will distribute the
*same* `analyze_event` across cores. When `Event` shared its columns through
`Rc` instead, this line did **not** compile — the compiler rejected the parallel
schedule until the data was made thread-safe. The fix wasn't "add locks"; it was
"satisfy the type that proves no data race exists."

## The unsafe schedule doesn't compile

The screencast then tries the classic mistake — filling one shared histogram bin
from every thread:

```rust
let hist = Rc::new(RefCell::new(0u64));   // one shared accumulator
(0..1_000_000).into_par_iter().for_each(|_| {
    *hist.borrow_mut() += 1;              // concurrent mutation: a data race
});
```

The compiler's verdict, verbatim:

```console
$ cargo build --example _race_demo
error[E0277]: `Rc<RefCell<u64>>` cannot be shared between threads safely
  ...
  = help: the trait `Sync` is not implemented for `Rc<RefCell<u64>>`
```

This is the whole thesis in one error. The data race is not a flaky test or a
Heisenbug found in production months later — it is a **compile error**, caught
before the program can run even once. In C++ this compiles and races; in Python
the GIL hides it and forbids the speedup outright.

## The safe schedule: a parallel reduce, ~8x, bit-identical

The correct form is a parallel *reduce* — each thread accumulates a local
`Summary`, then the partials merge associatively. Same kernel, real data:

```console
$ cargo run --release --example bench_parallel -- DoubleMuon_Run2016H_NANOAODv9.root 100000 50
nano.rust parallel demo read 100000 events into owned Event batch: 0.283s
correctness: serial == parallel outputs and reductions; input muons=214677, checksum=9593696.7
muon producer aggregate: selected_events=34777, n_good_muon=48921, lead_muon_pt_sum=3468780.710
serial   analysis+reduce x50: 1.548s (0.030950s/pass)
parallel analysis+reduce x50 on 16 rayon threads: 0.198s (0.003954s/pass), speedup=7.83x
```

Two assertions run *inside* the benchmark, not as prose: the parallel per-event
outputs equal the serial outputs, and the parallel reduction equals the serial
reduction — bit-for-bit (the `lead_pt_bits_checksum` compares raw float bits, so
even reordering-induced rounding would show up). On this 16-core machine that's a
**7.83x** speedup for a kernel nobody had to re-verify for thread safety: the
compiler already did.

## Why this matters for agentic analysis

This is the performance lever that falls out of the correctness argument for
free. An agent writes a per-event kernel; if it compiles against the spec, it is
*already* a kernel with no hidden shared-mutable state — which is exactly what
makes it safe to run event-at-a-time, column-at-a-time (SIMD), or chunk-parallel
across cores. You verify *what* once; the framework chooses *how*, and every
choice is correct by construction. Reproduce it with
`asciinema rec docs/site/demo-parallel.cast -c "bash scripts/demo_parallel.sh"`.
