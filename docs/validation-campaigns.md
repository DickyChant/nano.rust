# Validation Campaigns

Validation campaigns are the review contract above a single ROOT comparison.
An analysis spec defines event semantics; a campaign spec defines how that
analysis is proven ready for migration, branch development, or production use.

Campaign files live under `configs/validation/` and are checked with:

```bash
nano campaign configs/validation/wz_vbs_2024_v15_campaign.toml
```

The command does not run the full workflow yet. It validates that the campaign is
internally coherent: the analysis spec parses, the NanoAOD catalogue version is
known, implemented gates have their required fields, and root-compare gates do
not invent stochastic branches outside the analysis spec's declared
`[validation.compare]` policy.

## Campaign Shape

Use these sections:

- `[campaign]`: name, reviewed analysis spec, NanoAOD catalogue, and purpose.
- `[demo]`: optional thesis/demo framing for talks, docs, or agent tasks.
- `[[sample_slice]]`: bounded input sample set, source list, and event limits.
- `[[run]]`: executable skim/histogram production intent.
- `[[gate]]`: validation gates with `status = "implemented"` or `"planned"`.

Gate kinds currently recognized by the CLI are:

- `spec_static_validation`: the analysis spec validates against catalogue and
  correction payloads.
- `run_smoke`: the analysis runs on a bounded sample slice.
- `root_compare`: ROOT skim/reference comparison using a validation spec.
- `yield_closure`: selected-event yield closure over a materialized ROOT tree.

For `yield_closure`, declare `artifact`, `tree`, and `expected_entries`. If the
artifact exists where `nano campaign` is run, the command opens the ROOT file and
checks the tree entry count. If the artifact is external to the current machine,
the gate remains part of the campaign contract and is reported as
`not_checked_missing_artifact`; running the same campaign where the artifact is
materialized turns it into an actual pass/fail check.

## WZ VBS Demo

`configs/validation/wz_vbs_2024_v15_campaign.toml` is the first campaign-level
demo. It deliberately treats legacy ROOT parity as one gate, not the whole proof.
The campaign records the 20-event EOS smoke slice, the Rust interpreter output,
the legacy ROOT parity compare, an implemented selected-yield closure gate, and
the planned full-sample distribution closure needed before production use.

This matches the project thesis: physicists review specs and validation policy;
agents may modify implementation; Rust and validation reject inconsistent states.
