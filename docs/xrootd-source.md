# Remote `Source` (HTTP byte-range / xrootd)

Status: **HTTP byte-range = stage one, in progress** (promoted ahead of the semantic
layer so CI can read open data remotely with no stored files). Native xrootd remains a
later project.

## Confirmed: CMS Open Data is HTTPS byte-range readable

Probed `https://eospublic.cern.ch//eos/opendata/cms/Run2016H/DoubleMuon/.../*.root`:
- It returns **307** to an EOS data node (`:8443`) with a capability token in the URL;
  following the redirect (re-sending `Range`) yields **206 Partial Content** (verified:
  first 64 bytes = ROOT magic `root`; an independent range at a 1 MB offset also 206).
- TLS: the cert chain fails default verification (CERN grid CAs → "self-signed
  certificate in certificate chain"). Handle via a configurable CA bundle, or an
  opt-in insecure mode for public read-only data.
- Robust pattern: per `fetch(start,len)`, request the original eospublic URL with the
  `Range` header and follow redirects (the token may be per-request).

This composes with the bounded streaming reader: a remote read pulls only the baskets
each chunk touches → bounded memory, minimal bytes fetched. Implemented behind a
`http` cargo feature (pure-Rust, prefer `ureq`).

## TLS configuration

The HTTP source honors:

- `SSL_CERT_FILE=/path/to/ca-bundle.pem` to verify with an explicit CA bundle.
- `NANO_HTTP_INSECURE=1` to disable TLS certificate verification.

`NANO_HTTP_INSECURE=1` is intended only for public, read-only open data where the
caller accepts transport-authenticity risk. It must not be used for private data,
credentials, or write-capable endpoints. The ROOT reader still validates the ROOT
container structure and basket checksums where present, but insecure TLS no longer
authenticates the server.

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
