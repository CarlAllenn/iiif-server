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
| `install.sh` | GitHub release | `curl \| sh`, with this release's checksums baked in |

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
- **The Release PR's commit is created through the GitHub API**, not with
  `git commit`. This repository requires signed commits and a runner has no
  signing key, so a locally-made commit is `verified: false` and the Release
  PR cannot be merged at all — which is how v0.1.0 first failed. Commits
  made via the API are signed by GitHub with its own key.
  `createCommitOnBranch` also writes all three files in one commit, which the
  REST contents endpoint cannot. `open-release-pr.sh` asserts the resulting
  commit is verified rather than trusting that it is.
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

## Why the installer is hand-written rather than dist

`dist` (cargo-dist) is the obvious tool for a `curl | sh` installer, and it
cannot be used here for a concrete reason rather than a stylistic one.

To generate installers, dist must own the binary build — the installers
reference artifact names only it knows. But building `*-unknown-linux-musl`
on a runner needs a C toolchain targeting musl, because mimalloc is C
compiled through the `cc` crate (rustup's self-contained musl support covers
Rust linking, not arbitrary C). That means `musl-tools` via apt, which
harden-runner's `disable-sudo` forbids — correctly.

It would also cost a property worth keeping: the Linux binaries on the
release are *extracted from the container image*, so the binary someone
downloads is byte-identical to the one running in production. dist would
rebuild them.

The three convention collisions noted earlier — harden-runner cannot precede
checkout in its generated CI, it self-installs by piping curl to a shell, and
it brings a second rustup-managed toolchain — were survivable. The musl one
is not.

What replaced it is `make-installer.sh`, which generates the installer from
the artifacts that were actually produced, with each checksum baked in. That
last part matters: an installer that resolves "latest" at run time trusts
whatever the API returns at that moment, whereas this one can only install
the exact bytes the release published, and fails loudly otherwise. Verified
against a local server: happy path, corrupted download, and unsupported
platform.

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

## The first publish attempt, and what it cost

The first run of phase 2 failed at the push step: `IMAGE_NAME` came from
`${{ github.repository }}`, which preserves the owner's capitals, and OCI
repository names must be lowercase — the registry rejected
`ghcr.io/CarlAllenn/iiif-server` outright. Local testing never caught it
because local builds are tagged `iiif-server:local` and never touch a
registry.

Nothing was published. The image had built and passed its smoke test, and the
failure came before the push, so no digest, signature or transparency-log
entry ever existed. That is the architecture working: a broken publish leaves
nothing public to clean up.

Recovering it required suspending the tag-immutability ruleset to delete the
`v0.1.0` tag, then re-enabling it. That is a deliberate exception and worth
recording honestly:

- It was safe **only** because nothing referenced the tag — no release, no
  image, no signature, no consumer. The rule protects published artifacts,
  and there were none.
- The alternative was shipping 0.1.1 as the first release, leaving a
  generated `CHANGELOG.md` permanently announcing a 0.1.0 that does not
  exist. A public document that is wrong forever costs more than a
  documented, one-time suspension while the repository was empty.
- It should not happen again. Once anything is published, a tag is load
  bearing and the answer is a new version, not a rewritten one.

A workflow also runs the definition found at the ref that triggered it, which
is why the fix could not simply be re-run against the existing tag.

## Known gaps## Known gaps

- **The harden-runner allowlists in `release.yml` and `publish.yml` are
  derived by construction, not from an audit run.** Replace them with
  audit-derived endpoints after the first live release, per the CI canon.
- **Reproducible image builds are not yet asserted.** The inputs are pinned
  (base images by digest, dependencies by lockfile, `--locked`), but nothing
  proves two builds of a tag produce the same digest.
- **No Homebrew tap**, deliberately. A tap needs its own repository
  (`brew install user/tap/x` resolves only to `github.com/user/homebrew-tap`),
  and it would add packaging polish to a path `install.sh` already covers.
  The audience for this is GLAM institutions, where an evaluation runs on
  Docker and a systems team, not on a developer's Mac — so the tap earns
  little and costs a repository. Revisit if someone asks for it; the endgame
  for a tool with real adoption is homebrew-core, which needs no tap at all.
- **Release secrets are not scoped to a GitHub Environment.** zizmor's
  auditor persona flags this (`secrets-outside-env`); the repo gate runs at
  `pedantic`, which does not. Worth revisiting only if you want an approval
  gate on publishing — though the Release PR merge is already the human
  commitment point, so the second gate would mostly be ceremony.
