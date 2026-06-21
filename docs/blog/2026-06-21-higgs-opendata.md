# Reconstructing the Higgs on open data — identical to ROOT

*2026-06-21 — the flagship: a real, multi-channel analysis (ROOT's df103
Higgs→ZZ→4ℓ) ported to nano.rust, run on CMS Open Data over HTTPS, reconstructing
the 125 GeV Higgs peak — and matching ROOT's own result **bit-for-bit**.*

The earlier demos showed one cut or one spectrum. This is the real thing: a
complete analysis with three decay channels, lepton selection with track-quality
cuts, Z-candidate combinatorics, mass windows, and a four-lepton invariant mass —
faithfully ported from ROOT's `df103_NanoAODHiggsAnalysis` tutorial and run on the
public CMS Open Data 2012 samples.

![Higgs → ZZ → 4ℓ four-lepton mass, reconstructed by nano.rust from CMS Open Data — the peak at 125 GeV](../plots/higgs_m4l.png)

*The four-lepton invariant mass on the simulated signal — the Higgs at 125 GeV,
plotted with [kuva](https://crates.io/crates/kuva) straight from the Rust
analysis (`--plot`).*

<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.css">
<div id="player" style="margin:1.5rem 0"></div>
<script src="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.min.js"></script>
<script>
  AsciinemaPlayer.create('../demo-higgs.cast', document.getElementById('player'), {
    cols: 100, rows: 32, idleTimeLimit: 2, theme: 'asciinema', poster: 'npt:0:4'
  });
</script>

*(No player? Raw cast: [`demo-higgs.cast`](../demo-higgs.cast).)*

> **What this demo proves (and what it doesn't, yet).** This validates the
> pure-Rust **I/O and physics** against ROOT — bit-identical — and externalizes
> the analysis *knobs* to a [config](#the-config-that-steers-it). But the
> selection/reconstruction here is still hand-written Rust
> (`examples/higgs4l_opendata.rs`). The framework's deeper promise —
> *physicist writes a spec, the compiler-enforced kernel is **generated***, with
> no hand-written `.rs` to review — is demonstrated today on the
> [muon channel](2026-06-21-spec-to-code.html) (spec → validate → codegen →
> typestate kernel, proven equal to the reference). Making **this** Higgs
> analysis spec-driven (extending the semantic IR with 4ℓ combinatorics so the
> kernel is generated from a spec and proven bit-identical) is in progress.

## The analysis

For each of the **4μ**, **4e**, and **2e2μ** channels (`examples/higgs4l_opendata.rs`):

1. **Select leptons** — `nLepton` requirements, η/pt kinematics, isolation, and
   *track quality*: the 3-D impact-parameter significance `sip3d < 4`, `|dxy|`,
   `|dz|`, plus opposite-charge balance.
2. **Reconstruct two Z bosons** — form the opposite-charge same-flavor pair whose
   mass is closest to the Z (`reco_zz_to_4l`); the remaining pair is the second Z.
   Apply ΔR separation and the mass windows (Z₁ ∈ [40,120], Z₂ ∈ [12,120] GeV).
3. **Reconstruct the Higgs** — the invariant mass of the four selected leptons.

Each step maps directly to a df103 stage; the code is written to be *read* — it's
the human-reviewed physics, with the framework providing the typed, remote-on-demand
I/O underneath.

## The config that steers it

The physics *knobs* live in one TOML — `configs/higgs4l.toml` — not buried in the
code. A physicist reviews and edits this; the combinatoric kernel just runs it
(numbers stay bit-identical):

```toml
luminosity = 11580.0          # pb^-1 (11.6 fb^-1)

[selection.muon]              # per-flavour lepton selection
min_pt = 5.0
max_abs_eta = 2.4
max_pf_rel_iso04_all = 0.40
max_sip3d = 4.0               # 3-D impact-parameter significance
max_abs_dxy = 0.5
max_abs_dz = 1.0

[zcandidates]                 # Z reconstruction windows
z_reference_mass = 91.2
z1_mass_min = 40.0
z1_mass_max = 120.0
z2_mass_min = 12.0
z2_mass_max = 120.0

[histogram]
bins = 36
range = [70.0, 180.0]

[[sample]]                    # luminosity-weighted samples for the stack
name = "SMHiggsToZZTo4L"
role = "signal"
channels = ["4mu", "4e", "2e2mu"]
xsec = 0.0065
nevt = 299973.0
scale = 1.0

[[sample]]
name = "ZZTo4mu"
role = "background"
channels = ["4mu"]
xsec = 0.077
nevt = 1499064.0
scale = 1.386                 # ZZ k-factor
# … ZZTo4e / ZZTo2e2mu, and the Run2012 DoubleMu/DoubleElectron data samples …
```

Run it against any config with `--config`; the default is the file above. The
electron block, the data samples, and the per-channel mixed-flavour pt cuts are
in the same file — that's the *entire* physics surface a reviewer needs, separate
from the implementation.

## Identical to ROOT — bit for bit

The point of porting *ROOT's* tutorial: we can check against ROOT itself. Running
ROOT's df103 (`scripts/higgs4l_root_crosscheck.sh`) on the same skimmed signal,
every number matches:

| quantity | nano.rust (HTTPS) | ROOT (xrootd) |
|---|---|---|
| total selected 4ℓ | 26,708 | 26,708 |
| 4μ / 4e / 2e2μ | 9115 / 5528 / 12065 | 9115 / 5528 / 12065 |
| **120–130 GeV (Higgs peak)** | **23,370** | **23,370** |
| 110–120 GeV | 2080 | 2080 |
| 130–140 GeV | 647 | 647 |

Getting from "agrees to 0.01%" to *identical* meant matching ROOT's arithmetic
**precisely**: the impact-parameter significance (`ip3d`/`sip3d`) is computed in
the same float precision ROOT's `RVecF` uses, so the `sip3d < 4` cut flips
identically on the handful of boundary events; the invariant-mass and Z-pairing
arithmetic match ROOT's. A golden test now asserts these exact counts, so any
future drift fails CI.

## The full picture: signal + background + data

The plot above is the simulated signal alone. The real df103 result stacks the
luminosity-weighted signal and ZZ background and overlays the **2012 data** — the
actual discovery plot. `examples/higgs4l_stack_opendata` reads all eight skimmed
open-data samples over HTTPS, weights each by `lumi·σ/N` (lumi = 11.6 fb⁻¹), and
fills 36 bins over m(4ℓ):

![CMS Open Data H→ZZ→4ℓ: ZZ background, the m_H=125 signal stacked, and 2012 data — the Higgs discovery plot, reconstructed by nano.rust](../plots/higgs_stack.png)

The ZZ continuum and Z peak sit at low mass, the **m_H = 125 GeV signal bump**
rises above the background, and the **data points** track them — the four-lepton
Higgs excess, from public data, in pure Rust. Totals: signal 6.70, background
62.0, data 82. Against ROOT's df103 the agreement is to **f64 precision**
(~12 significant figures; data exact, signal per-bin identical, the background
sum differing only at ~1e-12 from summation order).

## Why this matters

This is the whole thesis, end to end, on a real analysis:

- a **complicated** analysis (three channels, track quality, Z combinatorics,
  mass windows, m4ℓ) — not a toy cut;
- on **public, reproducible** data, read **remotely on demand** in pure Rust (no
  ROOT, no download — ~6 MB fetched for the skimmed signal);
- **validated against the reference implementation** (ROOT) on its own tutorial,
  bit-for-bit;
- with a **publication-style plot** generated in-process (kuva).

The same typed I/O and event model that power this also power the spec → kernel →
workflow pipeline — so the framework's foundation is now proven against ROOT on
an analysis that reconstructs the Higgs boson.

## Reproduce it

```console
$ cargo run -p nano-io --example higgs4l_opendata --features full -- \
    "https://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/SMHiggsToZZTo4L.root" \
    --insecure --plot higgs.svg
```
