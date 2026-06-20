# Next-step project: remote `Source` (xrootd / HTTP byte-range)

Status: **planned**, not started. Sequenced after the semantic-layer slices.

## Why it's clean here

root-io's whole reader sits on one seam: `Source::fetch(start, len)` (local =
`File::open` + `seek`). A remote `Source` just implements `fetch` as a ranged
network read. It composes directly with the **bounded streaming reader** we now
have: reading a remote file would pull only the baskets each chunk touches —
**no full download**. That's grid/CMS data access (read a 2 GB file off
`root://…` while resident memory stays ~tens of MB), and it's the natural next
capability of "ROOT-on-demand."

So the architecture is ready; the work is the protocol.

## Options

| Option | Pure Rust | Effort | Notes |
|---|---|---|---|
| HTTP(S) byte-range (`Source::Http`) | yes | low | `reqwest`/`ureq` ranged GET. WLCG is moving toward HTTP access (DOMA). Site-dependent — e.g. `eospublic.cern.ch` did **not** answer https for the CMS open-data path tested (it served xrootd), so HTTP won't cover CMS open data as-is. |
| Native xrootd (`root://`), pure-Rust client | yes | high | No mature pure-Rust xrootd crate known. A minimal client for **public/unauthenticated** reads (handshake + `kXR_open` + `kXR_read`/`kXR_readv` + redirects) is feasible and scoped. GSI/token auth for non-public data is the hard part. |
| xrootd via FFI to `libXrdCl` | no (C++ dep) | medium | Battle-tested, full protocol + auth + redirects, but reintroduces the C++/system dependency we deliberately shed and needs XRootD installed. |
| `xrdcp` whole-file copy (current) | n/a | done | Works for validation; no on-demand random access. |

## Recommendation

1. Add a pure-Rust **`Source::Http`** byte-range backend first (cheap, useful wherever a site offers HTTPS+ranges; good for the DOMA direction).
2. Then a pure-Rust **`Source::Xrootd` for public reads** (open + readv + redirects, no auth) — the most on-thesis target for CMS open data; composes with the lazy reader for remote on-demand.
3. Keep **libXrdCl FFI** as the escape hatch for authenticated/grid data.

First, confirm whether a usable pure-Rust xrootd client already exists before
committing to writing one.

## Validation idea

Same file, two paths: read N events locally (downloaded via `xrdcp`) vs. read
the same N events via `Source::Xrootd`/`Source::Http` and assert identical
values; report bytes fetched (should be « file size) to prove on-demand.
