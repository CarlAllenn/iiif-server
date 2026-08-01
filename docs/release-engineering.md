# Release engineering

How this repository releases, and why it is shaped this way. Every rule below
was paid for by something that went wrong — here or in a sibling repo — and
the reasoning is recorded so a future change is a decision rather than a
rediscovery.

## The deliverables

| Artifact | Where | Who it is for |
| --- | --- | --- |
| Container image, multi-architecture | `ghcr.io/carlallenn/iiif-server` | deployment — the primary artifact |
| Static Linux binaries (amd64, arm64) | GitHub release | systemd, and institutions with no container platform |
| macOS binaries (Apple Silicon, Intel) | GitHub release | evaluation, and `iiif-server check` on a workstation |
| Validator report | GitHub release | conformance as verifiable fact, not a README claim |

Nothing is published to crates.io. See "Why not crates.io" below — it is a
standing decision, not a naming delay.

## Two phases, split by a tag

**Phase 1** (`release.yml`, on pushes to `main`) decides the version,
maintains the Release PR, and — when that PR is merged — tags and cuts a
**draft** GitHub release. It builds nothing and publishes nothing.

**Phase 2** (`publish.yml`, triggered by that tag) builds, publishes, proves,
signs and finally publishes the release, in a run whose `github.ref` *is* the
tag.

That split is the whole architecture, and the reason is provenance. An
attestation records the ref of the run that produced it. A workflow running on
`main` can only ever record `refs/heads/main` — a moving pointer that tells a
verifier nothing about which bytes were signed. Publishing from the tag makes
correct provenance a property of the shape rather than a hope. edtf learned
this the expensive way: its v1.0.0 attestations permanently name a commit that
built none of the published bytes, and Sigstore is append-only, so they cannot
be corrected.

Supporting rules:

- **Merging the Release PR is the commitment point.** Nothing releases on an
  ordinary push to main; an ordinary push only refreshes the PR.
- **Releases stay drafts until phase 2 finishes.** Immutability is applied
  when a release is *published*, not when it is created, so a release made
  public before its assets exist could never receive them. Drafts are also the
  better failure mode: a run that dies leaves nothing public.
- **The tag is pushed with a PAT**, not `GITHUB_TOKEN`. Tags pushed with the
  default token do not trigger workflows, and a release that silently
  triggers nothing looks exactly like a success.
- **Every phase-2 job refuses to run on a non-tag ref**, and refuses a tag
  whose version disagrees with the manifest.

## The publish invariant

Phase 2's step order is not rearrangeable:

> build → smoke test → push → pull the published bytes back and prove them →
> attest → verify the attestation names the tag → publish the release

**Attestation happens last, and only on proof.** A run that attests before
proving produces a signature that verifies green while asserting something
false — permanently.

Concretely: the image is smoke tested before it is pushed anywhere, because a
digest that anyone has pulled exists forever. After publication it is pulled
back *by digest*, smoke tested again, validated against the official IIIF
validators, and scanned, before cosign signs it. Then the signature is
verified the way a stranger would verify it — `cosign verify` against this
workflow's identity at this tag, and `gh attestation verify` with the tag as
source ref.

## Why phase 1 is git-cliff rather than a release tool

Both maintained options were tried against a scratch clone of this repository,
and both fail on the same underlying fact: **this workspace inherits its
version, and its crates are not published.**

**release-plz** determines what changed per crate by running `cargo package`,
which cannot succeed for interdependent crates absent from a registry:

- with `version = "x"` on the internal dependencies, cargo searches crates.io
  for `iiif-core` and fails;
- without it, cargo refuses outright — *"all dependencies must have a version
  requirement specified when packaging"*.

`release = false` on the library crates does not help, because `iiif-server`
itself is then unpackageable for the same reason. This is upstream issue
[#2595](https://github.com/release-plz/release-plz/issues/2595), open since
January 2026 and unfixed in 0.3.160, with the maintainer concluding the
compare operation itself has to change. Its shape is the dangerous part: **the
first release works, and the second one fails.**

**release-please** has no model for Cargo workspace inheritance at all. Its
`CargoWorkspace` type carries only `members`; its `CargoPackage` type models
`version` as a literal string; its updater throws on a virtual manifest
(*"is not a package manifest"*); and its `cargo-workspace` plugin ignores
`[workspace.dependencies]`
([#1896](https://github.com/googleapis/release-please/issues/1896), open) —
the same coupling that broke release-plz. Adopting it would mean restructuring
the workspace to suit the tool.

So phase 1 is `git-cliff` plus `gh`, in three small scripts. The version is
derived from conventional commits, never typed.

**Two configuration decisions in `cliff.toml` are load-bearing:**

- `breaking_always_bump_major = false`. git-cliff's default sends a 0.x
  breaking change straight to 1.0.0. In this project 1.0 is not a number, it
  is the [MAINTENANCE.md](../MAINTENANCE.md) scope-freeze commitment —
  feature-complete by design, security and correctness fixes only, forever.
  Reaching it because someone wrote `feat!:` before launch would publish a
  promise nobody made. 1.0.0 gets set by hand, once, deliberately.
- `no_increment_regex` excludes chore/ci/docs/style/test, so a quiet week
  produces no version and no empty Release PR.

`prepare-release.sh` keeps the **three** places that hold the version in
lockstep — `[workspace.package].version` and both `[workspace.dependencies]`
constraints — and fails loudly if any substitution misses, rather than opening
a PR whose tree cannot resolve.

## Why not crates.io

Publishing `iiif-core` and `iiif-sources` would make release-plz work. It is
refused, permanently, and not because of the naming question:

- Those crates exist to separate concerns inside one binary. They have no
  independent consumers, and the workspace split is an architectural choice,
  not a distribution one.
- Publishing them creates a **public Rust API with semver obligations**, on a
  project whose entire thesis is a minimal maintenance surface — and the
  compatibility promise this project actually makes covers the HTTP surface,
  the CLI flags and the container contract, explicitly not internal Rust APIs.
- crates.io names are permanent and unreclaimable. A GHCR path can be
  abandoned; a crate name cannot.

Revisit only if a real consumer asks — demand, not tooling convenience.

## Why the image is `FROM scratch`

The design spec called for it, and the obstacle turned out to be removable.

Verifying an S3 connection needs a trusted-certificate bundle. The spec chose
bundled `webpki-roots` precisely so a scratch image could do that — but
`reqwest` 0.13 removed the option entirely, and every rustls path now resolves
to `rustls-platform-verifier`, i.e. the operating system's trust store, which
a scratch image does not have. Rather than dependency surgery, the bundle
ships as a file with `SSL_CERT_FILE` pointing at it.

Worth knowing how that failure would have presented: serving local files works
fine without it. Only `s3://` deployments break.

## Why `cargo auditable` is not optional

Rust discards dependency information at compile time. A scanner pointed at a
stripped static Rust binary sees **one file and zero packages** — which is
exactly the zero-crates result monumental-archive hit when scanning an image
built from this repository's source.

`cargo auditable` embeds a compressed dependency list that syft, trivy and
grype all read. Verified rather than assumed: trivy identifies the binary as
`rustbinary`, and the SBOM lists 190 packages.

Two consequences:

- The build **must not strip** the binary. `strip` discards the non-allocated
  section the data lives in, silently undoing the SBOM.
- `sbom: true` on the image push is only meaningful because of this. On its
  own it would produce a durable, digest-attached, completely useless SBOM
  that *looks* like diligence.

## Owner-side prerequisites

These cannot be automated and gate the first release:

- **`RELEASE_TOKEN`** — a fine-grained PAT with contents and pull-requests
  write. Needed twice: PRs created with `GITHUB_TOKEN` do not trigger CI, and
  tags pushed with it do not trigger phase 2.
- **GHCR package visibility** set to public after the first publish.
- **Tag immutability ruleset** — forbid tag deletion, non-fast-forward and
  update, with no bypass actors. Every verification points at the tag; a
  movable anchor is no anchor.

## Known gaps

- **The harden-runner allowlists in `release.yml` and `publish.yml` are
  derived by construction, not from an audit run.** Replace them with
  audit-derived endpoints after the first live release, per the CI canon.
- **Reproducible image builds are not yet asserted.** The inputs are pinned
  (base images by digest, dependencies by lockfile, `--locked`), but nothing
  proves two builds of a tag produce the same digest.
- **Installers** — `curl | sh` and a Homebrew tap — are not yet published.
  Binaries are attested and checksummed on the release; the ergonomic layer
  on top is follow-up work.
