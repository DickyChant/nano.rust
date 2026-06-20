# Versioning policy

Different artifacts need *different kinds* of versioning. Treating the docs site
as one version is wrong — a blog post and a function signature do not move
together. This is the policy.

## Three kinds, one per artifact

| Artifact | Kind | What a reader wants | Mechanism |
|---|---|---|---|
| Blog / notes (`docs/blog/`) | **Editorial** | "when was this written / last revised?" | chronological date-slugs, immutable permalinks, *published* + *updated* dates, git history; an Atom feed |
| API / functions (rustdoc) | **Release** (semver) | "the docs for the version I depend on" | [docs.rs](https://docs.rs) per published crate version; or `/vX.Y/api/` rebuilt from `v*` tags |
| Spec / schema (`nano-spec`, catalogues) | **Compatibility** | "will my old spec still validate?" | a `schema_version` on `AnalysisSpec` + the NanoAOD catalogue version (`v9`/`v12`/`v15`) |

Underneath all three, every Pages deploy carries a **build stamp**
(`<ref> · <sha> · <date>`, in the footer) — a deploy fingerprint, not a version.

## Blog: editorial versioning

- **Immutable, time-ordered.** The slug `YYYY-MM-DD-title` is the permalink and
  never changes. A post is a dated artifact; it is not re-released.
- **Published vs updated.** *Published* is the slug date. *Updated* is derived
  automatically from the file's last commit date (`git log -1`); the site shows
  "updated …" only when it differs from published. So fixing a post (e.g. the
  fb⁻¹ unit correction) is visible without hand-maintained metadata.
- **History lives in git.** No per-post version numbers; `git log -- <file>` is
  the changelog.
- **Not coupled to code.** Blog versioning never tracks the crate version — an
  essay about a design idea stays valid across API releases.

> Requires full git history at build time: `docs.yml` checks out with
> `fetch-depth: 0` so per-file "updated" dates resolve (a shallow clone would
> date every file to the latest commit).

## API: release versioning

- **Semver, tracking the code.** `nano_spec::validate` in 0.1 may differ from
  0.2; API docs must be pinned to a release.
- **Published crates → docs.rs.** Once crates are published, docs.rs hosts
  versioned rustdoc for free, including the exact dependency versions. That is
  the canonical home for versioned API docs — prefer it over rolling our own.
- **Unreleased / pre-publish → tag-based rebuild.** Because Pages deploys a
  rebuilt artifact (no `gh-pages` branch), `build_site.py` can iterate `v*` tags
  and emit `/vX.Y/api/` for each, with `main` at the root as "latest" plus a
  `versions.json` + dropdown. This is dormant until the first tag.

## Spec: compatibility versioning

- The **catalogue version** (`v9`/`v12`/`v15`) already pins the branch universe a
  spec is validated against.
- The **spec grammar** (`[objects]`, `[regions]`, `[[model]]`, …) should carry a
  `schema_version` so the validator can accept or migrate older specs as the
  grammar evolves. Bump it only on breaking grammar changes; validation errors
  should name the expected schema version.

## Rule of thumb

Ask *what does a reader need to pin?* — a **date** (blog), a **release** (API),
or a **schema** (spec). Pin that, and only that. Never give one artifact another
artifact's version.
