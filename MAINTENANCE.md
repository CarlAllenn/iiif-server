# Maintenance policy

This server is **feature-complete by design and frozen**: from 1.0
onward it receives security and correctness fixes only, forever. This is
the respected "finished software" posture (TeX, qmail, the Go-1
compatibility promise, SQLite's 2050 pledge) — legitimate here because
the upstream spec is itself frozen: IIIF Image API 3.0 has been
unchanged since 2020, 2.1 since 2016, and image codecs are frozen file
formats. "Finished" is a reachable state, and this project has reached
for it deliberately.

## What is NOT claimed

Not a literal code-freeze. A network server parsing untrusted bytes
always needs security response and dependency bumps. The honest residual
is the **tracked dependency class** — `hyper`, `tokio`, `rustls`,
`object_store`, `tiff`, `zune-jpeg`, `png`, `j2k` — all pure Rust, which
demotes decoder advisories from RCE-class to mostly DoS-class. They are
handled as Renovate bumps with occasional 0.x API absorption: a handful of
interventions a year, mostly auto-merged. That — not "zero maintenance" —
is the claim.

Two corrections to an earlier version of that list, both found by
generating an SBOM from the shipped binary rather than by reading the
design spec. `webpki-roots` is not a dependency: `reqwest` 0.13 removed the
bundled-roots option and every rustls path now resolves to
`rustls-platform-verifier`, so the container image ships a certificate
bundle as a file instead — see
[docs/release-engineering.md](docs/release-engineering.md). And `moxcms` is
not a dependency yet, because ICC colour management is not implemented (#45), so
the class is eight crates today and nine when it lands.

Everything else (`fast_image_resize`, `jpeg-encoder`, `gif`,
`image-webp`, the hand-rolled PDF writer, all geometry math) is
**pin-forever**: pure compute that never sees hostile bytes and is never
forced to update.

## What a version number promises

Versions follow semver over a **stated surface**, because "follows semver"
means nothing until you say what it covers:

- **Covered:** the HTTP surface (IIIF endpoints, response semantics,
  headers), the CLI flags, and the container contract (entrypoint, default
  bind address, exposed port, user).
- **Not covered:** log format, metric values, and internal Rust APIs —
  nothing in this workspace is published to a registry, so there is no
  library API to promise anything about.

Tag policy: version tags are immutable; `0.1` and `0` float forward to the
newest release in their range; `latest` tracks the newest release. Pin a
digest for deployments and a version tag for everything else.

**1.0 is not a routine increment.** It is the scope-freeze commitment
above, set deliberately and once. Pre-1.0 a breaking change bumps the
minor version, and the release tooling is configured so that reaching
1.0.0 by accident is impossible.

## Response window

Security advisories against the tracked class are triaged within **7
days** and released as signed patch releases. The visible heartbeat —
merged Renovate PRs, signed patch releases, green scheduled CI — is what
tells adoption committees the stillness is intentional.

## Pre-refusals

Written down now so future refusals are quotes, not debates:

- **AVIF / JPEG-XL output: refused in advance.** Spec-legal via
  `extraFormats`, and exactly how scope-frozen projects die. We ship the
  spec's enumerated table; modern-format transcoding is the CDN layer's
  business.
- **Auth in the engine: refused.** Access control is the reverse proxy's
  job (`auth_request` / forward-auth pattern; recipe in
  [docs/deployment.md](docs/deployment.md)). The seam exists if reality
  overrules.
- **Presentation API, manifests, viewers: refused.** This engine is the
  Image API box only — the pixel layer. Manifests come from the
  application that owns the objects; viewers are embedded JavaScript
  consuming both.
- **`rights`/attribution in info.json: refused.** Per-image metadata
  requires per-image config, violating both zero-config and
  identical-info.json-for-every-image. Rights statements belong to the
  Presentation manifest owned by the application.

Also permanent non-goals: video/PDF *sources*, embedded scripting,
derivative caches, in-process TLS, lossy webp (requires C libwebp),
Image API v1, feature toggles of any kind.

## The security property

**Zero C parses untrusted input, anywhere in the product.** Request URLs
are hostile (fuzzed grammar and identifier resolution); source masters
are operator-curated and semi-trusted — a malicious master is an
availability problem, not RCE, because every decoder is memory-safe
Rust. The optional mimalloc allocator and the ring crypto cores are C
that computes over our own data and never parses hostile bytes; the
headline survives audit precisely because it is stated this carefully.
