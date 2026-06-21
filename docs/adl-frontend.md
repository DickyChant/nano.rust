# ADL as a front-end (design)

The physics-facing spec should be **more abstract** than today's TOML-with-
expression-strings. The right vehicle is **ADL** (Analysis Description Language)
— the established, physics-native declarative language for collider analyses
(objects, regions, derived variables, built-in functions like `m()`, `dR`,
`size`, `sum`, and combinatorics with operators like *closest-to-mass*). It's in
the vision already (`ADL/YAML/TOML/DSL`).

## The one idea: ADL is just another front-end

`docs/architecture.md` is the whole reason this is clean: **front-end → typed IR
→ back-ends.** ADL becomes another front-end that parses into the *same*
`ResolvedPlan` IR that TOML/YAML do. Everything below — `validate`, codegen
(typestate kernel), interpret, JIT, the workflow DAG — is unchanged.

```
muon.toml ─┐
muon.adl  ─┼─parse→ AnalysisSpec ─validate→ ResolvedPlan ─→ codegen/interpret/JIT → kernel
muon.yaml ─┘                                          (one IR, all back-ends)
```

**Status: this is a design/plan — the ADL parser is not built yet** (see the plan
below); today the front-end is TOML/YAML.

**What we adopt:** ADL's *syntax and object model* (the abstraction physicists
already know). **What we do NOT adopt:** ADL's interpreter/runtime (CutLang). The
plan is to map ADL onto our **harness** — so an ADL analysis *would* inherit the
same validator+rustc guarantees the TOML front-end gets (and the
bit-identical-to-ROOT self-check) once the parser lowers ADL into the same IR.
ADL/CutLang don't provide that hard guarantee. *That
combination — ADL abstraction + a hard compiler gate — is the novel contribution.*

## Why ADL is more abstract than our TOML

Today (ad-hoc, verbose, repeated sub-expressions):
```toml
require = [
  "all(good_muon, sqrt(dxy*dxy + dz*dz)/sqrt(dxyErr*dxyErr + dzErr*dzErr) < 4.0)",
  "closest_mass(z1, z2, 91.2 GeV) > 40 GeV",
  "closest_mass(z1, z2, 91.2 GeV) < 120 GeV",
]
```
ADL (named defines, built-ins, combinatorics, chained comparisons):
```
define sip3d = sqrt(dxy^2 + dz^2) / sqrt(dxyErr^2 + dzErr^2)
object goodMuons : Muon
  select Pt > 5 and abs(Eta) < 2.4 and abs(pfRelIso04_all) < 0.4 and sip3d < 4
object Z1 : comb(goodMuons, 2)  select OSSF  nearest 91.2
object Z2 : comb(goodMuons, 2)  select OSSF  exclude Z1
region SR4mu
  select size(goodMuons) == 4
  select sum(goodMuons.Charge) == 0
  select 40 < m(Z1) < 120  and  12 < m(Z2) < 120
```
Named `define`s remove repetition; `m()/dR/size/sum`, `comb(...)`, `OSSF`,
`nearest`, and chained `a < x < b` are physics-native. It maps directly onto the
IR primitives we already built (pair/nested candidates, `nearest_mass`,
`closest_mass`/`other_mass`, `all()`/`count()`/reductions, arithmetic `define`s).

## Mapping ADL → our IR (it already lines up)

| ADL | our IR (built) |
|---|---|
| `object G : Coll select <pred>` | `ObjectDef` with cuts |
| `define x = <expr>` | named derived scalar (arithmetic `Expr`) |
| `comb(G,2) select OSSF nearest M` | `derived` pair, `opposite_charge`, `nearest_mass{M}` |
| `... exclude Z1` | derived `exclude` |
| `region R select <preds>` | `RegionDef` (`all()`/`count()`/reductions) |
| `m(Z1)`, `dR(a,b)`, `size`, `sum` | invariant mass / `min_delta_r` / count / sum |
| combined Z1+Z2 → H | `derived` `combine` |

The IR is already expressive enough for the dimuon and (per-channel) Higgs
analyses, so an ADL subset can target it now.

## Plan (first slice, then grow)

1. **ADL-subset parser** in `nano-spec` (new `adl` module): `object` / `define` /
   `region` blocks, the built-ins above, `comb`/`nearest`/`exclude`/`OSSF`,
   chained comparisons → `AnalysisSpec` (the same type TOML produces). Add `.adl`
   to `SpecFormat`/`from_path`.
2. **Prove it**: re-express the muon and dimuon analyses in `.adl`; assert the
   ADL-derived `AnalysisSpec`/`ResolvedPlan` equals the TOML one, and that codegen
   from the ADL spec is **bit-identical** to the TOML/hand-written kernel (reuse
   the gen-demo equivalence discipline). Then the 4μ Higgs in ADL.
3. **CLI/MCP**: `validate`/`branches`/`codegen`/`run` accept `.adl` transparently
   (format by extension); `nano` gains nothing new — ADL is just an input format.
4. **Grow** toward fuller ADL (histograms/bins/tables, weights/systematics
   blocks, multi-region) as the IR grows; track ADL-feature coverage honestly.

Deliberately deferred: full ADL grammar/CutLang parity, ADL `table`/`bins`
histogramming, and importing existing CutLang analyses verbatim. Start with the
subset that covers our analyses, mapped onto the typed IR, with the bit-identical
proof as the gate.

## Honest framing

This makes the *physicist-facing* layer standard and abstract (ADL) while keeping
the *guarantee* (the Rust compiler) — i.e. it raises the abstraction without
loosening the harness. It does not replace the IR or the back-ends; it's a nicer
door into the same compiler-enforced building.
