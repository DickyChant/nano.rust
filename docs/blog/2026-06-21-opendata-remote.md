# Reading CMS Open Data over HTTPS, on demand, no download

*2026-06-21 — a screencast: pull the first events out of a 2 GB CMS Open Data
NanoAOD file straight from the CERN open-data server, in pure Rust, fetching
~1.3 MB and storing nothing.*

A recurring friction in HEP analysis is *getting the data to the code* — staging
multi-GB files before you can touch a single event. nano.rust's owned ROOT I/O
reads **remotely on demand**: it issues HTTPS byte-range requests for only the
baskets it actually needs. So "open this 2 GB file and give me 5 events" fetches
kilobytes, not gigabytes, with no local copy.

This is the real thing — the file is a public CMS Open Data NanoAODv9 file on
`eospublic.cern.ch`, and the read is the pure-Rust `nano-io` reader (no ROOT, no
`xrootd`).

<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.css">
<div id="player" style="margin:1.5rem 0"></div>
<script src="https://cdn.jsdelivr.net/npm/asciinema-player@3.8.0/dist/bundle/asciinema-player.min.js"></script>
<script>
  AsciinemaPlayer.create('../demo-opendata.cast', document.getElementById('player'), {
    cols: 100, rows: 30, idleTimeLimit: 2, theme: 'asciinema', poster: 'npt:0:3'
  });
</script>

*(No player? Raw cast: [`demo-opendata.cast`](../demo-opendata.cast).)*

## What you're seeing

```console
$ read_url_json "https://eospublic.cern.ch//eos/opendata/cms/Run2016H/DoubleMuon/.../*.root" 5 --insecure
  run=281616 event=59740  nMuon=0 Muon_pt=[]
  run=281616 event=172857 nMuon=1 Muon_pt=[...]
  ...
  fetched 1,323,577 bytes of 2,016,828,178  =  0.066% of the file
```

Real Run2016H events (run `281616`), read by streaming only the baskets touched:
**1.3 MB of a 2 GB file — 0.066%.** No file was downloaded or stored; the
`--insecure` flag is only for the EOS grid TLS chain, not the read itself.

How it works, briefly: `nano-rootio` opens the file via an HTTP `Source` that
serves `Range` requests; the TKey/TTree/TBranch metadata is read first (a couple
of small ranges), then each requested branch's baskets are fetched lazily as the
event iterator advances. The `_meta.bytes_fetched` vs `file_size` in the output
is measured, not estimated.

## It's the same reader, validated

This isn't a special "remote mode" — it's the same `nano-io` reader with an HTTP
source instead of a file source (`events_url` / `events_url_chunked` behind the
`http` feature). And it is **value-validated against `uproot`** on this exact
open-data file in CI on every push (`scripts/bench_vs_uproot.py`): our remote
read and uproot's agree event-for-event. So the convenience (no staging) comes
with no correctness asterisk.

## Reproduce it

```console
$ cargo run -p nano-io --example read_url_json --features http -- \
    "https://eospublic.cern.ch//eos/opendata/cms/Run2016H/DoubleMuon/NANOAOD/UL2016_MiniAODv2_NanoAODv9-v1/2510000/127C2975-1B1C-A046-AABF-62B77E757A86.root" \
    5 --insecure
```

The same capability is what lets CI read open data with **no checked-in data
files**, and it's the on-ramp to running the whole pipeline — selection,
weights, skim — directly against remote datasets.
