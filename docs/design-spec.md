# Design Spec — Modern IIIF Image Server

**Status:** DECIDED — scoping session 2026-07-26, amended the same day by a full inline dependency evaluation (crate sources downloaded and inspected, official spec/compliance documents and validator status re-verified). The only open items are the two code-gated M0 spikes and the product name.
**Naming:** the repo name `iiif-server` is a deliberate placeholder. Product name, GitHub org, and domain are deferred to the launch milestone (M8). The constraint this imposes: **publish nothing to crates.io, Docker Hub, or any registry until named** — registry names are forever. The GitHub repo itself is **public from M0** (decided 2026-07-26, CI economics: public repos get unlimited free Actions minutes including the free arm64 runners our x86/ARM matrix wants; a later rename/transfer redirects cleanly, unlike registries). **Public ≠ announced:** no candidate product names in docs, issues, branches, or commits before M8; no announcement before M8.
**Audience:** a fresh session (or contributor) should be able to run tooling standup and the build from this document alone.

---

## Mission

A complete, correct, boring implementation of the IIIF Image API — **3.0 level 2 and 2.1 level 2, each with the entire optional feature table** — as a single static binary with a maintenance floor that is low, honest, and precisely characterized. The engine delivers the spec, nothing more. **Zero C code parses untrusted input, anywhere in the product.**

The structural bet, stated precisely: **the spec surface is frozen** — Image API 3.0 unchanged since 2020, 2.1 since 2016, image codecs are frozen file formats. The dependency surface is *not* frozen and we do not pretend it is; instead it is split into two classes (below) so that forced change is confined to a small, named set. Therefore "finished" is a reachable state, not an aspiration — this server can genuinely be *done* in a way almost no software can.

Contrast exhibit: Cantaloupe's 6.0 milestone — 36 issues of pure platform debt (Spring Boot replatform, dead-dependency escapes like JAI, codec-interaction bugs, four actual conformance fixes), delivering roughly nothing new to an end user. The spec never moved; its foundations did. This project inverts those choices so no such milestone can ever be needed.

## Conformance target — the complete build

**1.0 ships the entire IIIF Image API 3.0 compliance table: level 2 plus every optional feature**, with exactly one asterisk (webp, below). Verified against the spec and compliance documents as pulled 2026-07-26.

| Surface | 1.0 ships | Notes |
|---|---|---|
| Region | `full`, `square`, `x,y,w,h`, `pct:x,y,w,h` | Complete |
| Size | `max`, `w,`, `,h`, `pct:n`, `w,h`, `!w,h` and **all `^` upscaling forms** | Upscaling supported ⇒ spec requires `maxWidth`/`maxArea` published — we always publish limits anyway (DoS posture) |
| Rotation | Arbitrary floating-point degrees + mirroring (`!n`) | Core milestones ship 90° steps + mirroring; arbitrary rotation lands in the completionist sweep (M6), pure computation, zero new deps |
| Quality | `default`, `color`, `gray`, `bitonal` | Complete |
| Output formats | `jpg`, `png` (core, the L2 requirement), then `tif`, `jp2`, `gif`, `pdf`, `webp` in M6 | tif: `tiff` crate already present · jp2: `j2k` encode path · gif: small pure-Rust crates · pdf: ~150-line hand-rolled single-image wrapper · **webp: lossless-only via `image-webp`** (encoder verified present in 0.2.4 source) — the single asterisk: valid `image/webp`, larger files, documented in one README sentence. No lossy webp because that requires C libwebp |
| Source formats | Pyramidal/tiled TIFF, JP2 + HTJ2K, plain JPEG, PNG | What real collections hold; working JP2 is the differentiator vs incumbents — and HTJ2K arrives free with `j2k`, a capability no OpenJPEG-based incumbent has. Full matrix in "Source-format matrix" below |
| HTTP (required at L2) | CORS + OPTIONS preflight · JSON-LD content negotiation for info.json (with `Vary`) · base-URI → info.json redirect | Conformance items, not polish — they belong to M1 |
| HTTP (optional) | HEAD · canonical `Link rel="canonical"` · profile `Link` header · float-formatting canonicalization rules | All cheap, all shipped |
| info.json | All required props · `tiles` and `sizes` **derived from the actual pyramid structure** · `maxWidth`/`maxHeight`/`maxArea` always published | Accurate tiles/sizes make viewers request only natively-cheap tiles — an underrated performance feature incumbents fumble |
| v2.1 endpoint | Full translation layer over the same engine — **all 18 named v2.1 features**, enumerated from the official compliance document: `regionByPx`, `regionByPct`, `regionSquare`, `sizeByW`, `sizeByH`, `sizeByPct`, `sizeByWh`, `sizeByConfinedWh`, `sizeByDistortedWh`, `sizeAboveFull`, `rotationBy90s`, `rotationArbitrary`, `mirroring`, `baseUriRedirect`, `canonicalLinkHeader`, `profileLinkHeader`, `cors`, `jsonldMediaType` | `full`↔`max` aliasing, profile-array info.json shape, `@id` vs `id`, `sizeAboveFull` mapped to the engine upscale path, `sizeByDistortedWh` (v2-only, dropped in v3) is non-aspect-preserving `w,h` — trivial for the resampler. Mounted at `/iiif/2/`, v3 at `/iiif/3/`. Both validator-locked |

**Why full-table is optimal, not maximalist:** every optional feature is either pure math (rotation, `square`, `^` forms, bitonal, canonicalization) or encode-only output (gif, pdf, webp-lossless, tif, jp2) — all in the pin-forever dependency class. The forced-maintenance surface of the full-table build is **identical** to a bare-L2 build; the decoders set the floor and we need those anyway. Same cost, far stronger claim.

**Error semantics (spec-defined, engine-enforced):** 400 malformed/out-of-range · 404 unknown identifier · 501 unsupported feature variant · 503 overload (backpressure, with `Retry-After`) · 401/403 belong to the proxy layer, not the engine.

**Capability is baked in, not toggled.** The binary supports exactly one honest feature set; info.json is generated from that fact, identical for every image. No feature knobs, no way for a deployment to misdeclare itself or fall out of conformance. The only deployment-varying values are the numeric limits.

## Stack

- **Rust stable**, MSRV stated in README and enforced in CI (floor set by `j2k`, currently 1.96).
- **HTTP: `hyper` 1.x directly + `tokio`. No web framework.** Rationale: our routing *is* the IIIF grammar parser we build anyway (two endpoint families); CORS/HEAD/conditional-request handling are a few honest lines each; hyper and tokio carry real multi-year 1.x stability promises. (Note the corrected claim: hyper/tokio are the *only* 1.x-promised crates in the tree — avoiding axum removed one churn source, not the last one. The two-class doctrine below is what actually bounds churn.)
- **Pure-Rust image pipeline — no libvips, no glib tree, no system libraries, no C toolchain:**
  - Decode: `tiff` (pyramidal/tiled; delegates JPEG-in-TIFF to zune-jpeg), `zune-jpeg`, `png`, `j2k`
  - Encode: `jpeg-encoder`, `png`, `tiff`, `gif` (+ quantization), hand-rolled single-image PDF, `image-webp` (lossless)
  - Resample: `fast_image_resize` (SIMD)
  - ICC color: **`moxcms`** — DECIDED. Actively maintained, `forbid(unsafe_code)` when built without SIMD feature flags. `qcms` rejected: dormant on crates.io since early 2024, 103 raw unsafe sites.
- **JP2/HTJ2K: `j2k` — pure Rust, DECIDED as primary (source-verified 2026-07-26).** The engine crate `j2k-native` is `#![forbid(unsafe_code)]` *including* its SIMD (via `fearless_simd`); MIT OR Apache-2.0; dep tree is `rayon`, `libm`, `log` + internal crates. Its API is the IIIF access pattern verbatim: tile decode, ROI decode, reduced-resolution decode, and `decode_region_scaled_into` — region-at-scale in one call. Decodes HTJ2K (Part 15) as well as classic JP2.
  - **Risk register (eyes open):** young (0.7.x, single org `frames-sg`, low download count) · input model is `&[u8]` — mmap for local files; object-store JP2 means full-object fetch or a bounded source-chunk cache (recorded in Architecture) · internal rayon parallelism must be pinned to our worker-pool sizing via its `CpuDecodeParallelism` setting · decomposition-level metadata for info.json `tiles`/`sizes` may need our own ~50-line SIZ/COD marker parse if not exposed.
  - **Fallback ladder:** all codecs sit behind a trait in `core`. Plan B is vendored-FFI OpenJPEG (battle-tested, upstream CVE tracking, costs a C toolchain and the zero-C headline) — a contained swap, verifiable against the differential goldens. The `openjp2` c2rust transpile is **dropped**: unsafe-riddled like the C it was translated from, but with CVE fixes lagging upstream — the worst of both paths.
- **Sources: `object_store`** (Apache Arrow project) — DECIDED, coverage over purity. Local filesystem + S3-compatible endpoints (custom endpoint URLs; Hetzner Object Storage is a day-one test case) + GCS + Azure behind one small trait. We consume a three-call surface (`get`, `get_range`, `head`), so its 0.x breaking majors are mechanical renames absorbed at bump time; in exchange it owns the credential swamp (IMDS, IRSA, SAS, workload identity). A hand-rolled S3 client was evaluated and rejected.
- **TLS roots: `rustls` + bundled `webpki-roots`** — the only shape compatible with a `FROM scratch` image (no system root store). Root-store refreshes are routine Renovate items.
- **Allocator:** musl's malloc is a known multithreaded-workload hazard and the M2 bench must not measure it by accident. M0 benches musl-native vs `mimalloc`; if contention shows, ship mimalloc — which is C, classified honestly as **trusted-compute C that never parses hostile bytes**. The precise headline either way: *zero C parsing untrusted input.*
- **Observability:** `tracing` structured logs · `/healthz` · minimal hand-rolled Prometheus text `/metrics` (fixed set: request counts, latency histogram, worker-queue depth, 503 count; zero new deps, frozen surface). Decided now because scope-freeze makes it permanent either way.
- **Workspace:** `core` (pure library: grammar, pipeline, info.json, codec trait — the grammar layer does no I/O), `server` (binary), `sources` (object_store wrapper + identifier resolution). **`#![forbid(unsafe_code)]` workspace-wide** — the JP2 boundary crate from the original plan is deleted; it existed to contain unsafe, and there is none left to contain. Dual-licensed MIT/Apache-2.0.

### Dependency doctrine — the two classes

A dependency forces work only if it **(a)** parses hostile or semi-trusted bytes (security advisories, forever) or **(b)** churns underneath us (0.x breaking majors, absorbed at bump time). Sorting the tree by that metric:

| Class | Members | Policy |
|---|---|---|
| **Tracked** (security watch, forever) | `hyper`, `tokio`, `rustls`+`webpki-roots`, `object_store`, `tiff`, `zune-jpeg`, `png`, `j2k`, `moxcms` | Renovate + advisory response within the MAINTENANCE.md window. Nine crates, all pure Rust — memory safety demotes decoder advisories from RCE-class to mostly DoS-class |
| **Pin-forever** (no hostile bytes, pure compute) | `fast_image_resize`, `jpeg-encoder`, `gif`, `image-webp`, PDF writer, all geometry/rotation/canonicalization math | Pinned at known-good versions; never forced to update. `fast_image_resize` being at major v6 is irrelevant — nobody sends an attack payload through a resampler |

Honesty note, on the record: apart from hyper/tokio, the tracked class is 0.x (versions as inspected 2026-07-26: tiff 0.11.3, zune-jpeg 0.5.15, png 0.18.1, j2k 0.7.5, moxcms 0.9.0, object_store 0.14.1). The committed lockfile makes churn opt-in; the steady state is a handful of interventions a year, mostly auto-merged. That — not "zero maintenance" — is the claim, and it survives audit.

## Architecture

- **Stateless; no derivative cache.** Correct `ETag` / `Cache-Control` / conditional-request semantics; derivatives immutable. Derivative caching is the CDN/reverse-proxy's job. (Cantaloupe's derivative-cache machinery is a pre-CDN artifact.)
  - **One precise carve-out, decided now so it can't be lawyered later:** a **bounded in-memory source-metadata cache** (TIFF headers/IFDs/tile indexes, JP2 headers, optionally bounded JP2 source chunks). This is not derivative caching: the CDN can never cache source range-reads, and without this cache every tile request re-pays several sequential object-store round trips before the first pixel moves. Bounded, in-memory, evicting — no disk, no invalidation protocol.
- **The source-read seam is a founding interface, not an M4 discovery:** `core` defines an async byte-range trait (`read_range`/`length`) from M0. Decoders are sync and bridge at the boundary; local files satisfy it via mmap/read, `object_store` via ranged GETs with coalescing. This exists precisely so M4 is "add a backend," not "rework core I/O."
- **No TLS in-process.** Terminate at the proxy/CDN; documented deployment recipe.
- **No auth in the engine.** Access control = reverse proxy (`auth_request` / forward-auth pattern), documented as a recipe. Seam exists; revisit post-1.0 only if reality demands.
- **Bounded blocking-worker pool** for decode/resample with explicit queue depth; overflow → 503 + `Retry-After`. Pool size and queue depth are the two tuning knobs of the DoS posture, alongside always-published `maxWidth`/`maxHeight`/`maxArea` and per-decode allocation/pixel-count ceilings (decompression-bomb guards). `j2k`'s internal rayon pool is pinned to this sizing.
- **Threat model, explicit:** source masters are operator-curated (semi-trusted; a malicious master is an availability problem, not RCE, because every decoder is memory-safe Rust) · request URLs are hostile (fuzzed grammar + identifier resolution) · resource exhaustion is the primary live threat (limits + backpressure above). This is why zero-C-parsing-untrusted-input is a security property, not an aesthetic.
- **Identifier resolution is a named security component:** exactly one percent-decode pass (spec rules: encoded slashes and `/ ? # [ ] @ %`), no re-decode, canonical-path traversal rejection. Directly fuzzed.
- **Near-zero config:** `<binary> serve ./images` or `<binary> serve s3://bucket/prefix --endpoint https://…` just works; env-var overrides. The source layer takes a **prefix→root map whose default size is one** — so if multi-root reality arrives, the answer is documentation, not architecture (pre-decided to defuse the most likely config-growth pressure). Anti-pattern on record: Cantaloupe's ~200-key properties file.
- **ETag definition (M5):** hash of (source identity + source version [store ETag or mtime+size], canonical request URI, binary version). Cheap, correct, no state.
- **`<binary> check` subcommand:** offline master inspection — warns "this TIFF isn't tiled/pyramidal and will serve slowly," flags old-style JPEG-in-TIFF (tag 6 — cleanly unsupported by the decoder, pre-1995 format), prints the one-line conversion fix; can advise HTJ2K transcode for faster JP2 serving. Converts the incumbent's #1 support burden into setup-time advice. Operator tooling, not spec surface.

## Source-format matrix (M2 acceptance criteria)

"Complete" implementations die by unstated pixel-format edge cases, so the matrix is stated and closed:

**Supported:** tiled/pyramidal TIFF — none/deflate/LZW/ModernJPEG(tag 7) compression, shared `JPEGTables` (verified in tiff 0.11.3 source; JPEG path delegates to the zune-jpeg we ship anyway — zero added deps), 8/16-bit, chunky + planar, YCbCr incl. subsampled · JP2 and HTJ2K via `j2k` (incl. native bit depths, palette, subsampled components per its metadata API) · plain JPEG incl. CMYK/YCCK (zune-jpeg) · PNG incl. palette, 16-bit, gray+alpha. BigTIFF: expected supported, confirmed by M2 goldens.

**Rejected, cleanly, with `check`-time advice:** old-style JPEG-in-TIFF (tag 6) · anything outside the matrix. A rejected master produces one actionable error message, never a wrong image.

## Quality regime

- **Spec-derived property tests** on URL grammar and canonicalization (including float-formatting rules); parse↔print round-trips.
- **Golden-tile corpus** with perceptual hashing — codec/resample regressions cannot land silently.
- **Differential JP2 testing — an advantage no incumbent has:** three independent implementations (`j2k`, `hayro-jpeg2000` as pure-Rust second opinion, OpenJPEG as golden reference) vote on the same corpus; disagreement is a test failure and a fuzz oracle.
- **Official IIIF validators** (v3 + v2) wired as a task and run in CI, **pinned from the `IIIF/image-validator` git repo** (active, last pushed 2025-10) — the PyPI package is stale (2019). **Validator output published as a release artifact** — conformance as verifiable fact, not README claim. The validator is the conformance *floor*: one leg of three (validator = spec conformance, goldens = pixel correctness, property tests = grammar correctness).
- **Fuzzing as the security posture:** `cargo-fuzz` targets for URL parser, identifier resolution, and every decoder boundary; local burn-in at M7; **OSS-Fuzz enrollment lands after M8** — it requires the public name (moved from M7; enrolling under a placeholder then renaming is churn in a third-party repo).
- **Supply chain (the monumental-archive gate discipline):** committed lockfile · `cargo-deny` (advisories + licenses) · digest-pinned CI images · reproducible builds · SBOM per release · cosign-signed releases · `FROM scratch` image containing one static binary.
- **Benchmark honesty, with numbers:** M2 benches against a libvips reference on a stated corpus (pyramidal-TIFF and JP2 tile workloads) on fixed, documented hardware. Gate: **p50 ≤ 1.5× libvips, p99 ≤ 2×**. Miss → Plan B evaluation scoped to the failing codec only (vendored OpenJPEG for JP2, libvips wholesale only as last resort). The M0 allocator bench exists so this gate measures our pipeline, not musl's malloc.

## Maintenance policy — scope-freeze, stated loudly

At 1.0 the feature set is **complete by design and frozen**: security and correctness fixes only, forever. This is the respected "finished software" posture (TeX, qmail, the Go-1 compatibility promise, SQLite's 2050 pledge) — uniquely legitimate here because the upstream spec is itself frozen.

Not claimed: literal code-freeze. A network server parsing untrusted bytes always needs security response and dependency bumps. The honest residual is the nine-crate tracked class above — advisories demoted by memory safety from RCE-class to mostly DoS-class, handled as Renovate bumps with occasional 0.x API absorption.

**Finished must not look abandoned.** Ship `MAINTENANCE.md` declaring: feature-complete by design; security/correctness releases only; advisory response within a stated window. The visible heartbeat — merged Renovate PRs, signed patch releases, green scheduled CI — is what tells adoption committees the stillness is intentional.

**Pre-refusals, written down now so future refusals are quotes, not debates** (these go in MAINTENANCE.md verbatim):

- **AVIF / JPEG-XL output:** refused in advance. Spec-legal via `extraFormats`, and exactly how scope-frozen projects die. We ship the spec's enumerated table; modern-format transcoding is the CDN layer's business.
- **Auth in the engine:** refused. Forward-auth recipe covers it; the seam exists if reality overrules.
- **Presentation API, manifests, viewers:** refused. Layering (below).
- **`rights`/attribution in info.json:** refused. Per-image metadata requires per-image config, violating both zero-config and identical-info.json-for-every-image; rights statements belong to the Presentation manifest owned by the application. Documented rationale ships with the refusal.

## Milestones

- **M0 — skeleton + spikes + the seam.** Workspace (3 crates, workspace-wide `forbid(unsafe_code)`), CI (Linux x86/ARM), licenses, tooling standup (mise, Task, Renovate, lefthook, cargo-deny). Typed URL grammar, property-tested. **The async byte-range source trait defined in `core` from day one.** info.json for one local TIFF. One real tile decoded → resized → encoded. Official validator wired from git. Allocator bench (musl vs mimalloc). **Object-store mini-spike:** S3 range-read latency profile against Hetzner; metadata-cache sizing data. **SPIKE 1 (reduced scope — capability already confirmed from tiff 0.11.3 source):** correctness/perf goldens for JPEG-in-TIFF pyramid tiles, esp. subsampled YCbCr. **SPIKE 2 (redefined):** `j2k` vs OpenJPEG goldens — byte-level correctness and the region-at-scale perf gate on multi-gigapixel pyramidal JP2s; verify decomposition-level metadata exposure (else write our ~50-line SIZ/COD parse); pin rayon to pool sizing. Failure → vendored-FFI OpenJPEG behind the codec trait (documented Plan B; cheap reversal at this stage only).
- **M1 — v3 level-2 conformance, one source format.** Full grammar, canonical URIs, CORS/conneg/base-redirect, error semantics. Validator green.
- **M2 — source-format matrix.** The full matrix above; ICC via moxcms; golden corpus + differential JP2 rig established; libvips bench against the stated gates.
- **M3 — v2.1 endpoint.** Translation layer; all 18 features; v2 validator green.
- **M4 — object_store sources.** S3-compatible custom endpoint (Hetzner) as a first-class test; GCS/Azure by construction; metadata cache tuned with M0 spike data. (The seam already exists — this milestone adds backends, not architecture.)
- **M5 — HTTP caching correctness.** ETags (definition above), conditionals, immutability — headers, not machinery.
- **M6 — completionist sweep.** Arbitrary rotation; `tif`/`jp2`/`gif`/`pdf`/`webp-lossless` outputs — each landing with its goldens and fuzz targets. After M6 the entire compliance table is shipped.
- **M7 — hardening.** Local fuzz burn-in, limit tuning, load-tested backpressure.
- **M8 — naming + packaging + launch.** Name sweep and decision; org creation/transfer; static binaries; scratch image; trust bundle (SBOM, reproducible-build verification, cosign, validator artifact); docs and deployment recipes (proxy auth, CDN, TLS); **OSS-Fuzz enrollment under the real name**. First public announcement happens here and not before.

## Non-goals (permanent unless reality overrules)

Presentation API · viewers (embed Mirador/OpenSeadragon/UV; never build) · manifest generation · video or PDF *sources* (page extraction is the ingesting application's job, done once at ingest — never live in the serving path) · embedded scripting of any kind · derivative caches · in-process TLS · auth logic · lossy webp (would require C libwebp) · AVIF/JPEG-XL outputs (pre-refused above) · per-image metadata in info.json · Image API v1 · feature toggles.

**Layer discipline reminder:** IIIF is a family. This engine is the Image API box only — the pixel layer. Manifests (Presentation API) come from the application that owns the objects; viewers are embedded JavaScript consuming both. Deep-linking citations to pages/regions (canvas URIs, `#xywh`, Content State) is manifest/application territory and works with any conformant image server, including this one.

## Open items

1. **SPIKE 1 (M0):** JPEG-in-TIFF correctness/perf goldens (capability confirmed from source 2026-07-26).
2. **SPIKE 2 (M0):** `j2k` correctness + region-at-scale performance vs OpenJPEG goldens; levels-metadata check; rayon pinning. Plan B: vendored-FFI OpenJPEG behind the codec trait.
3. **Allocator bench (M0):** musl-native vs mimalloc under concurrent decode.
4. **Product name / org / domain** — parked until M8 by explicit decision. Until then: no registry publication, no announcement.

## Appendix — evaluation record (2026-07-26)

Crate sources downloaded from crates.io and inspected directly; findings above rest on these versions: `j2k`/`j2k-native` 0.7.5 · `hayro-jpeg2000` 0.4.0 · `dicom-toolkit-jpeg2000` 0.5.0 (eliminated: whole-frame only) · `openjp2` 0.6.1 (dropped) · `tiff` 0.11.3 · `zune-jpeg` 0.5.15 · `png` 0.18.1 · `image-webp` 0.2.4 (encoder verified) · `moxcms` 0.9.0 (selected) · `qcms` 0.3.0 (rejected) · `fast_image_resize` 6.1.0 · `jpeg-encoder` 0.7.0 · `gif` 0.14.2 · `object_store` 0.14.1 · `hyper` 1.11.0 · `tokio` 1.53.1. Validator: `IIIF/image-validator` git (pushed 2025-10; PyPI package stale at 2019). v2.1 feature names enumerated from `iiif.io/api/image/2.1/compliance/`.
