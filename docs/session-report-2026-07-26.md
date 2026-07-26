# Build report — overnight session, 2026-07-26

Starting point: the founding spec and four docs commits. No code.
Ending point: a working, validator-conformant, fuzz-hardened IIIF Image
API server covering the spec's engineering milestones M0–M7.

## Conformance, verified not claimed

Official IIIF validator, run in CI on every push, reports published as
build artifacts:

- **Image API 3.0, level 2: 33/33 tests, zero failures**
- **Image API 2.1, level 2: 30/30 tests, zero failures**

Local test suite: 94 tests across grammar property tests, identifier
security tests, per-codec pixel tests, output-format round-trips,
rotation semantics, and HTTP semantics — plus 9 fixture-gated spike
tests. `task check` (fmt + clippy pedantic + cargo-deny + seven other
linters + tests) is green, with **zero `allow` attributes anywhere in
the workspace**.

## What the differential/fuzz testing caught

Three real defects, each found by a mechanism the spec asked for:

1. **YCbCr color bug (SPIKE 1).** For ModernJPEG chunks the `tiff` crate
   keeps zune-jpeg's *input* colorspace, so `read_chunk` returns raw
   Y′CbCr. Treating it as RGB measured mean |Δ| = 89.6/255 against the
   libjpeg golden. Fixed; 4:4:4 is now exact, 4:2:0 shows only
   upsampling-filter skew (mean 0.37).
2. **Upstream `j2k` region-decode bug.** On any JP2 tile grid with
   partial edge tiles, region decode returns wrong pixels — even for
   interior tiles — while whole-image decode is bit-exact. Detected by
   comparing against OpenJPEG goldens. Contained with a
   decode-full-then-crop fallback, pinned by fixtures for both paths,
   and surfaced to operators by `iiif-server check`. Worth reporting
   upstream. *(Done: filed as
   [frames-sg/j2k#62](https://github.com/frames-sg/j2k/issues/62) with a
   standalone reproducer, 2026-07-26.)*
3. **25 GB decompression bomb (fuzzing).** A 68-byte PNG header
   declaring 512×16777335 allocated 25 GB before a pixel was decoded — a
   trivial remote OOM. The spec's per-decode pixel ceiling is now
   enforced at every whole-decode boundary; the exact input is a
   committed regression fixture.

## Spike outcomes (all four M0 spikes resolved)

| Spike | Verdict |
| --- | --- |
| SPIKE 1 — JPEG-in-TIFF | PASS after the color fix; 21.9 ms for a 48-tile level |
| SPIKE 2 — `j2k` vs OpenJPEG | PASS: lossless region decode **bit-exact**; HTJ2K bit-exact and 2.7× faster; metadata exposed (no SIZ/COD hand-parse needed); rayon pinnable |
| Allocator bench | mimalloc **1.5–2.0×** faster than musl malloc under concurrent decode → ships as default |
| Object-store profile | Cold open = 3 sequential round trips (45–90 ms at real RTT) — quantifies exactly what the M4 metadata cache must amortize |

Details in `docs/spikes/`.

## The one gate that failed

The M2 libvips benchmark, run honestly (CLI startup floor measured and
subtracted; hardware and corpus printed):

- pyramidal TIFF: **0.22–0.54× libvips** — we are 2–4× faster
- JP2 region decode: **2.01× p50** — outside the 1.5× gate

Adaptive codec parallelism (lend cores when the pool is idle, keep them
when saturated — measured crossover) took JP2 from 3.21× to 2.01×. The
remaining gap is the wavelet decode itself. The spec's rule is
"miss → Plan B evaluation scoped to the failing codec," and Plan B
(vendored OpenJPEG) would cost the zero-C headline — a product decision,
deliberately left open with three options and a recommendation in
`docs/bench/libvips-gate.md`.

## Deviations from the spec worth knowing

- **Fixture tooling lives in `tools/fixtures/mise.toml`**, not the root
  config: mise's conda backend writes no Linux lock entries from a macOS
  host, which broke CI under locked mode. Exact pins and the 72 h
  cooldown remain; the product toolchain stays fully locked.
- **`object_store` is built with `aws-base` + ring**, not the packaged
  `aws` feature, so the outbound-TLS crypto provider is an explicit
  `init_tls()` call rather than a transitive aws-lc default. The
  doctrine question (is TLS record parsing "trusted compute C"?) is
  written down, not silently resolved.
- **JP2/JPEG/PNG masters are read whole**; only TIFF streams through the
  byte-range seam. This matches the spec's acknowledged `&[u8]` model
  for `j2k`; the bounded chunk cache remains the recorded refinement.

## Open items for you

1. **JP2 performance** — accept 2×, take Plan B, or wait on upstream
   (`docs/bench/libvips-gate.md` recommends accepting for now).
2. **CLA-assistant GitHub app** — needs your OAuth; the CLA text ships.
3. **Naming, registries, signed releases, OSS-Fuzz** — all correctly
   still parked at the launch milestone.
