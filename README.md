# iiif-server (working name)

[![ci](https://github.com/CarlAllenn/iiif-server/actions/workflows/ci.yml/badge.svg)](https://github.com/CarlAllenn/iiif-server/actions/workflows/ci.yml)

A complete, correct, boring implementation of the [IIIF Image
API](https://iiif.io/api/image/3.0/) — **3.0 and 2.1, level 2 plus the
entire optional feature table** — as one static binary. Pure Rust
including JPEG 2000/HTJ2K decode; **zero C parses untrusted input
anywhere in the product**; stateless; scope-frozen at 1.0.

```bash
iiif-server serve ./images
```

```bash
iiif-server serve s3://bucket/prefix --endpoint https://objects.example.com
```

That is the whole configuration story: one root, numeric limits, pool
sizing. No properties file, no feature toggles — capability is baked in
and info.json is generated from what the binary actually does, identical
for every image.

## What ships

| Surface | Status |
| --- | --- |
| Image API 3.0, level 2 + all optional features | official validator: 33/33 |
| Image API 2.1, all 18 named features | official validator: 30/30 |
| Regions: `full`, `square`, px, `pct:` | complete |
| Sizes incl. every `^` upscaling form | complete |
| Rotation: 90° steps, mirroring, arbitrary angles | complete (transparent corners on PNG/WebP) |
| Qualities: `default`, `color`, `gray`, `bitonal` | complete |
| Outputs: `jpg png tif gif jp2 pdf webp` | complete (webp lossless-only — the one asterisk¹) |
| Sources: pyramidal/tiled TIFF (incl. JPEG-in-TIFF), JP2 + HTJ2K, plain JPEG/PNG | complete |
| Local filesystem + S3-compatible object stores | complete (GCS/Azure by construction) |
| ETags, conditionals, CORS, content negotiation, canonical links | complete |
| Bounded decode pool with honest backpressure (503 + Retry-After) | complete |
| `/healthz`, Prometheus `/metrics` | complete |
| `iiif-server check` — offline master inspection with copy-paste fixes | complete |

¹ Lossy webp requires C libwebp; valid `image/webp` is served losslessly
instead, at larger byte sizes. That is the compliance table's single
footnote.

## Why it stays finished

The spec surface is frozen (3.0 unchanged since 2020, 2.1 since 2016;
codecs are frozen file formats), so *complete* is reachable — and after
1.0 the feature set never grows. What remains forever is a nine-crate
tracked dependency class, all pure Rust, handled as routine bumps. The
full doctrine, response window, and **pre-refusals** (AVIF/JXL, auth,
Presentation API, per-image metadata — declined in advance, with
rationale) live in [MAINTENANCE.md](MAINTENANCE.md).

Correctness is enforced three ways, continuously: the **official IIIF
validators** run in CI on every push (reports published as artifacts),
**golden/differential tests** pin pixels against libvips, libjpeg, and
OpenJPEG — bit-exact where the math says bit-exact — and **property
tests** cover the grammar (parse↔print round-trips, canonicalization,
totality). The differential/fuzz rig has caught and contained three
real defects before any user could hit them — two upstream decoder
bugs and a 25 GB decompression bomb
([session report](docs/session-report-2026-07-26.md)).

How this stacks up against the incumbent server — latency,
conformance, and ops, measured with the same no-subtractions
methodology and including where the incumbent wins — is in
[docs/bench/cantaloupe-eval.md](docs/bench/cantaloupe-eval.md).

## Building and developing

Toolchain is pinned with [mise](https://mise.jdx.dev); MSRV is Rust 1.96.

```bash
mise install
task ci       # exactly what CI runs: fmt + all linters + tests
task validate # official IIIF validators (both API versions), local build
```

The workspace is `#![forbid(unsafe_code)]` throughout, clippy pedantic
with zero `allow` attributes, and every dependency is permissively
licensed (enforced by `cargo deny`). See
[CONTRIBUTING.md](CONTRIBUTING.md) (external contributions need the
[CLA](CLA.md)) and [docs/design-spec.md](docs/design-spec.md) — the
founding document this build follows.

Deployment recipes (CDN caching, forward-auth, systemd):
[docs/deployment.md](docs/deployment.md).

## Status

The founding spec's engineering milestones are built and continuously
verified. Product naming, registry publication, signed releases, and the
first announcement are deliberately deferred to the launch milestone —
the repo being public is not the launch.

Licensed [AGPL-3.0-only](LICENSE).
