# Design Spec — Modern IIIF Image Server

**Status:** DECIDED — outcome of the scoping session, 2026-07-26. The only open items are the two M0 spikes and the product name.
**Naming:** the repo name `iiif-server` is a deliberate placeholder. Product name, GitHub org, and domain are deferred to the launch milestone (M8). The only constraint this imposes: **publish nothing to crates.io, Docker Hub, or any registry until named** — repo renames/transfers leave permanent redirects, registry names are forever.
**Audience:** a fresh session (or contributor) should be able to run tooling standup and the build from this document alone.

---

## Mission

A complete, correct, boring implementation of the IIIF Image API — **3.0 level 2 and 2.1 level 2** — as a single static binary with a maintenance floor approaching zero. The engine delivers the spec, nothing more.

The structural bet: **everything upstream is frozen.** Image API 3.0 unchanged since 2020, 2.1 since 2016; image codecs are frozen file formats; every load-bearing Rust dependency carries a formal 1.x stability promise. Therefore "finished" is a reachable state, not an aspiration — this server can genuinely be *done* in a way almost no software can.

Contrast exhibit: Cantaloupe's 6.0 milestone — 36 issues of pure platform debt (Spring Boot replatform, dead-dependency escapes like JAI, codec-interaction bugs, four actual conformance fixes), delivering roughly nothing new to an end user. The spec never moved; its foundations did. This project inverts those choices so no such milestone can ever be needed.

## Conformance target — the complete build

**1.0 ships the entire IIIF Image API 3.0 compliance table: level 2 plus every optional feature**, with exactly one asterisk (webp, below). Verified against the spec and compliance documents as pulled 2026-07-26.

| Surface | 1.0 ships | Notes |
|---|---|---|
| Region | `full`, `square`, `x,y,w,h`, `pct:x,y,w,h` | Complete |
| Size | `max`, `w,`, `,h`, `pct:n`, `w,h`, `!w,h` and **all `^` upscaling forms** | Upscaling supported ⇒ spec requires `maxWidth`/`maxArea` published — we always publish limits anyway (DoS posture) |
| Rotation | Arbitrary floating-point degrees + mirroring (`!n`) | Core milestones ship 90° steps + mirroring; arbitrary rotation lands in the completionist sweep (M6), pure computation, zero new deps |
| Quality | `default`, `color`, `gray`, `bitonal` | Complete |
| Output formats | `jpg`, `png` (core, the L2 requirement), then `tif`, `jp2`, `gif`, `pdf`, `webp` in M6 | tif: `tiff` crate already present · jp2: OpenJPEG already contained · gif: small pure-Rust crates · pdf: ~150-line hand-rolled single-image wrapper · **webp: lossless-only via pure Rust** — the single asterisk: valid `image/webp`, larger files, documented in one README sentence. No lossy webp because that requires C libwebp, and the zero-new-C doctrine wins |
| Source formats | Pyramidal/tiled TIFF, JP2, plain JPEG, PNG | What real collections hold; working JP2 is the differentiator vs incumbents |
| HTTP (required at L2) | CORS + OPTIONS preflight · JSON-LD content negotiation for info.json · base-URI → info.json redirect | Conformance items, not polish — they belong to M1 |
| HTTP (optional) | HEAD · canonical `Link rel="canonical"` · profile `Link` header · float-formatting canonicalization rules | All cheap, all shipped |
| info.json | All required props · `tiles` and `sizes` **derived from the actual pyramid structure** · `maxWidth`/`maxHeight`/`maxArea` always published | Accurate tiles/sizes make viewers request only natively-cheap tiles — an underrated performance feature incumbents fumble |
| v2.1 endpoint | Full translation layer over the same engine | `full`↔`max` aliasing, profile-array info.json shape, `@id` vs `id`, 2.1 `sizeAboveFull` mapped to the engine upscale path. Mounted at `/iiif/2/`, v3 at `/iiif/3/`. Both validator-locked |

**Error semantics (spec-defined, engine-enforced):** 400 malformed/out-of-range · 404 unknown identifier · 501 unsupported feature variant · 503 overload (backpressure) · 401/403 belong to the proxy layer, not the engine.

**Capability is baked in, not toggled.** The binary supports exactly one honest feature set; info.json is generated from that fact, identical for every image. No feature knobs, no way for a deployment to misdeclare itself or fall out of conformance. The only deployment-varying values are the numeric limits.

## Stack

- **Rust stable**, MSRV stated in README and enforced in CI.
- **HTTP: `hyper` 1.x directly + `tokio`. No web framework.** Rationale: axum is 0.x with a history of breaking majors — the one framework-churn source in the candidate tree — while hyper 1.x carries an explicit multi-year stability promise. Our routing *is* the IIIF grammar parser we build anyway (two endpoints per API version); CORS/HEAD/conditional-request handling are a few honest lines each. After this choice, **every load-bearing upstream has a formal stability commitment.**
- **Pure-Rust image pipeline — no libvips, no glib tree, no system libraries, no C toolchain:**
  - Decode: `tiff` (pyramidal/tiled), `zune-jpeg`, `png`
  - Encode: `jpeg-encoder`, `png`, `tiff`, `gif` (+ quantization), hand-rolled single-image PDF, `image-webp` (lossless)
  - Resample: `fast_image_resize` (SIMD)
  - ICC color: `qcms` or `moxcms` (picked during M0)
- **JP2: contained OpenJPEG, the only foreign code in the product.** SPIKE 2 decides delivery: `openjp2` (c2rust-transpiled port — cargo-only build, true `cargo install`) vs thin FFI over vendored, pinned OpenJPEG source. Isolated in its own boundary crate either way.
- **`#![forbid(unsafe_code)]`** on `core` and `server`; unsafe exists only inside the JP2 boundary crate.
- **Sources: `object_store`** (Apache Arrow project) — local filesystem + S3-compatible endpoints (custom endpoint URLs; Hetzner Object Storage is a day-one test case) + GCS + Azure behind one small trait. Not the AWS SDK (smithy tree, wrong shape).
- **Observability: `tracing`** structured logs; `/healthz`.
- **Workspace:** `core` (pure library: grammar, pipeline, info.json — the grammar layer does no I/O), `server` (binary), `jp2` (boundary crate). Dual-licensed MIT/Apache-2.0.

**"Zero dependencies," stated precisely:** no system libraries (true static musl binary, `FROM scratch` image), no C except one contained vendored OpenJPEG, and a small pure-Rust cargo tree auditable via the committed lockfile.

## Architecture

- **Stateless; no internal cache subsystem.** Correct `ETag` / `Cache-Control` / conditional-request semantics; derivatives immutable. Caching is the CDN/reverse-proxy's job. (Cantaloupe's derivative-cache machinery is a pre-CDN artifact.)
- **No TLS in-process.** Terminate at the proxy/CDN; documented deployment recipe. Removes the cert/config surface entirely.
- **No auth in the engine.** Access control = reverse proxy (`auth_request` / forward-auth pattern), documented as a recipe. Revisit post-1.0 only if reality demands, behind a seam, never in the spec-faithful core.
- **Bounded blocking-worker pool** for decode/resample (CPU-bound sync work) with explicit queue depth; overflow → spec-sanctioned 503. Pool size and queue depth are the two tuning knobs of the DoS posture, alongside always-published `maxWidth`/`maxHeight`/`maxArea` and per-decode allocation/pixel-count ceilings (decompression-bomb guards).
- **Identifier resolution is a named security component:** exactly one percent-decode pass (spec rules: encoded slashes and `/ ? # [ ] @ %`), no re-decode, canonical-path traversal rejection, one configured root (directory or bucket+prefix). Directly fuzzed.
- **Near-zero config:** `<binary> serve ./images` or `<binary> serve s3://bucket/prefix --endpoint https://…` just works; env-var overrides; a config file only if the surface ever earns one. Anti-pattern on record: Cantaloupe's ~200-key properties file.
- **`<binary> check` subcommand:** offline master inspection — warns "this TIFF isn't tiled/pyramidal and will serve slowly," prints the one-line conversion fix. Converts the incumbent's #1 support burden into setup-time advice. Operator tooling, not spec surface.

## Quality regime

- **Spec-derived property tests** on URL grammar and canonicalization (including float-formatting rules); parse↔print round-trips.
- **Golden-tile corpus** with perceptual hashing — codec/resample regressions cannot land silently.
- **Official IIIF validators** (v3 + v2) wired as a task and run in CI; **validator output published as a release artifact** — conformance as verifiable fact, not README claim.
- **Fuzzing as the security posture:** `cargo-fuzz` targets for URL parser, identifier resolution, and every decoder boundary; **enroll in OSS-Fuzz** so the fuzzing runs continuously on external compute forever.
- **Supply chain (the monumental-archive gate discipline):** committed lockfile · `cargo-deny` (advisories + licenses) · digest-pinned CI images · reproducible builds · SBOM per release · cosign-signed releases · `FROM scratch` image containing one static binary.
- **Benchmark honesty:** M2 includes a bench against a libvips reference; pure-Rust must be within striking distance on the tile-serving workload or Plan B triggers.

## Maintenance policy — scope-freeze, stated loudly

At 1.0 the feature set is **complete by design and frozen**: security and correctness fixes only, forever. This is the respected "finished software" posture (TeX, qmail, the Go-1 compatibility promise, SQLite's 2050 pledge) — uniquely legitimate here because the upstream spec is itself frozen.

Not claimed: literal code-freeze. A network server parsing untrusted bytes always needs security response and dependency bumps (pure-Rust decoder advisories are the honest residual — demoted by memory safety from RCE-class to mostly DoS-class, and handled as automated Renovate bumps).

**Finished must not look abandoned.** Ship `MAINTENANCE.md` declaring: feature-complete by design; security/correctness releases only; advisory response within a stated window. The visible heartbeat — merged Renovate PRs, signed patch releases, green scheduled CI — is what tells adoption committees the stillness is intentional.

## Milestones

- **M0 — skeleton + spikes.** Workspace, CI (Linux x86/ARM), licenses, tooling standup (mise, Task, Renovate, lefthook, cargo-deny). Typed URL grammar, property-tested. info.json for one local TIFF. One real tile decoded → resized → encoded. Official validator wired. **SPIKE 1:** `tiff` crate vs JPEG-compressed pyramid tiles (the common real-world master layout). **SPIKE 2:** `openjp2` transpile vs vendored-FFI OpenJPEG. Spike failure → libvips is the documented Plan B (cheap reversal at this stage only).
- **M1 — v3 level-2 conformance, one source format.** Full grammar, canonical URIs, CORS/conneg/base-redirect, error semantics. Validator green.
- **M2 — source-format matrix.** Pyramidal TIFF, JP2, JPEG, PNG; ICC handling; golden corpus established; libvips bench.
- **M3 — v2.1 endpoint.** Translation layer; v2 validator green.
- **M4 — object_store sources.** S3-compatible custom endpoint (Hetzner) as a first-class test; GCS/Azure by construction.
- **M5 — HTTP caching correctness.** ETags, conditionals, immutability — headers, not machinery.
- **M6 — completionist sweep.** Arbitrary rotation; `tif`/`jp2`/`gif`/`pdf`/`webp-lossless` outputs — each landing with its goldens and fuzz targets. After M6 the entire compliance table is shipped.
- **M7 — hardening.** Fuzz burn-in, OSS-Fuzz enrollment, limit tuning, load-tested backpressure.
- **M8 — naming + packaging + launch.** Name sweep and decision; org creation/transfer; static binaries; scratch image; trust bundle (SBOM, reproducible-build verification, cosign, validator artifact); docs and deployment recipes (proxy auth, CDN, TLS). First public announcement happens here and not before.

## Non-goals (permanent unless reality overrules)

Presentation API · viewers (embed Mirador/OpenSeadragon/UV; never build) · manifest generation · video or PDF *sources* (page extraction is the ingesting application's job, done once at ingest — never live in the serving path) · embedded scripting of any kind · internal caches · in-process TLS · auth logic · lossy webp (would require C libwebp) · Image API v1 · feature toggles.

**Layer discipline reminder:** IIIF is a family. This engine is the Image API box only — the pixel layer. Manifests (Presentation API) come from the application that owns the objects; viewers are embedded JavaScript consuming both. Deep-linking citations to pages/regions (canvas URIs, `#xywh`, Content State) is manifest/application territory and works with any conformant image server, including this one.

## Open items

1. **SPIKE 1 (M0):** `tiff` crate JPEG-in-TIFF tile support.
2. **SPIKE 2 (M0):** JP2 via `openjp2` transpile vs thin vendored FFI.
3. **Product name / org / domain** — parked until M8 by explicit decision. Until then: no registry publication, no announcement.
