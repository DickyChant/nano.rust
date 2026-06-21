# nano-validate

Golden-validation harness for comparing a produced skim ROOT file against a
frozen reference ROOT file.

`nano_validate::compare_root_files(reference, candidate, options)` opens both
files with `nano-rootio`, reads the requested TTree, matches branches by name,
and compares every value in overlapping branches. Branches are reported as:

- `present_in_both`
- `only_in_reference`
- `only_in_candidate`

For each overlapping branch, the structured serde report includes the value
kind, ROOT type names, values compared, mismatches, maximum absolute and
relative differences, and the first few mismatching entries. Integer and boolean
branches compare exactly. Floating branches pass when:

```text
abs(reference - candidate) <= atol + rtol * abs(reference)
```

The CLI wrapper is:

```sh
nano compare <reference.root> <candidate.root> --tree Events --rtol 1e-6 --atol 1e-6
nano --json compare <reference.root> <candidate.root>
```

The command prints the same report shape as the library. It exits with status 0
for `pass` and non-zero for `fail`.

## Muon Golden References

This crate provides the comparison infrastructure for the frozen C++/ROOT
muon-validation outputs under `tests/data/muon_validation/references/*.root`.
Those files are full-analysis reference outputs. The matching input NanoAOD
files that produced them are not shipped in git, so the full end-to-end
validation is ready to run only after those inputs are placed locally under the
gitignored `tests/data/muon_validation/inputs/` directory.

Existing frozen references:

- `singlemu_2018_nanov9_reference.root`
- `ttbarfl_2022EE_nanov12_reference.root`
- `ttbarfl_2024_nanov15_reference.root`
- `ttbarsl_2016APV_nanov9_jer_down_reference.root`
- `ttbarsl_2016APV_nanov9_jer_up_reference.root`
- `ttbarsl_2016APV_nanov9_jes_down_reference.root`
- `ttbarsl_2016APV_nanov9_jes_up_reference.root`
- `ttbarsl_2016APV_nanov9_lheweight_reference.root`
- `ttbarsl_2016APV_nanov9_met_down_reference.root`
- `ttbarsl_2016APV_nanov9_met_up_reference.root`
- `ttbarsl_2016APV_nanov9_reference.root`
- `ttbarsl_2016_nanov9_reference.root`
- `ttbarsl_2017_nanov9_reference.root`
- `ttbarsl_2018_nanov9_reference.root`
