# Worked Example: Full Analysis Workflow

This example is the smallest end-to-end analysis path that still looks like a
real Combine production job. It lives at
`crates/nano-io/examples/full_analysis_workflow.rs` and runs entirely on
synthetic in-memory NanoAOD-like events.

Run it with:

```bash
cargo run -p nano-io --example full_analysis_workflow
```

The sample table is embedded in the example. It uses `10 FbInv` and five sample
rows: one `signal` sample, two `ttbar` samples that accumulate into one
background process, one `zjets` background sample, and one `data_obs` data
sample. MC rows use the production normalization formula from
`samples.rs`, `xsec * lumi / sumw`, with cross sections written with units.

The analysis spec is also embedded. It uses the normal `nano-spec` interpreter
path with:

- a JSON lumi mask from `crates/nano-spec/tests/data/synthetic_golden.json`
- `good_muon` and `good_jet` object selections
- one signal region requiring trigger, good vertices, at least one selected
  muon, and at least one selected jet
- one `signal_region` leading-jet-pt histogram
- the synthetic correctionlib muon scale factor payload
- the synthetic correctionlib JES uncertainty payload

The example writes `datacard.txt` and `shapes.root` under a temporary
`/tmp/nano_io_full_analysis_workflow_*` directory and prints the per-process
nominal yields:

```text
Per-process nominal yields:
  signal: 1.025000 bins=[0.54, 0.0, 0.485]
  ttbar: 4.220000 bins=[2.655, 1.08, 0.485]
  zjets: 0.852000 bins=[0.42000000000000004, 0.43200000000000005, 0.0]
  data_obs: 4.180000 bins=[2.13, 1.08, 0.97]
```

The emitted `datacard.txt` is:

```text
imax 1 number of channels
jmax 2 number of processes minus 1
kmax 2 number of nuisance parameters
------------
shapes * * shapes.root $CHANNEL/$PROCESS $CHANNEL/$PROCESS_$SYSTEMATIC
------------
bin signal_region
observation 4.18
------------
bin signal_region signal_region signal_region
process signal ttbar zjets
process 0 1 2
rate 1.025 4.22 0.852
------------
JesTotal shape 1 1 1
MuonSf shape 1 1 1
```

The integration test `crates/nano-io/tests/full_analysis_workflow.rs` runs the
same example workflow, checks the hand-computed process rates, validates the
datacard rows, and reads `shapes.root` back through ROOT I/O.
