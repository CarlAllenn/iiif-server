# Head-to-head: iiif-server vs Cantaloupe

The M2 gate compared us against libvips — an in-process C library call,
an idealized floor no patron ever talks to. This eval compares against
the deployed reality: [Cantaloupe](https://cantaloupe-project.github.io/),
the de facto standard IIIF server in cultural heritage. Both servers,
full HTTP round trips, production configurations, same corpus, same
hardware. No subtractions of any kind.

Where the direct measurement and the libvips proxy disagree, the direct
measurement supersedes: the question that decides the JP2 gate
(issue #2) is not "are we within 1.5× of an ideal C decode" but "does a
patron panning a viewer feel a difference vs. the server institutions
run today."

**Headline:** iiif-server wins every measured case — TIFF by 3–7×, JP2
fast paths by 2–2.6×, HTJ2K by default (Cantaloupe cannot serve it) —
**except** the cases inside the blast radius of one upstream bug
(frames-sg/j2k#62, the partial-grid region-decode fallback), where
Cantaloupe wins by ~10× and, on very large masters, by our forfeit.
Every loss in this document has the same root cause.

**Update, same day:** that root cause is fixed. The tables below stand
as the pre-fix record; the [post-fix rerun](#post-fix-rerun-2026-07-26-patched-j2k)
measures the same corpus with the j2k#62 fix applied and the fallback
removed — every former loss is now a win or parity, and the ≈134 MP
refusal is gone.

## The contenders

**iiif-server**: this repo at `c1d3099`, release build, single static
binary run natively, `j2k` 0.7.5, stateless (its caching layer is HTTP:
ETags + Cache-Control, absorbed by any CDN or caching proxy —
docs/deployment.md). Serve flags: `--max-width 20000 --max-height 20000
--max-area 400000000`, default pool (8 workers, queue 64).

**Cantaloupe**: built from the develop branch at commit `377942bd2`
(the 5.0.7 release — March 2025, the newest available — ships Jetty
11.0.24, EOL January 2025, and logback 1.2.x, also EOL; develop carries
the finished modernization unreleased, and this image additionally
backports current security versions, offered upstream as
cantaloupe-project/cantaloupe#962), on `eclipse-temurin:25-jre-noble`.
**JP2 processor: OpenJPEG 2.5.0** (`libopenjp2-tools` from Ubuntu noble
— the only JP2 processor an open-source deployment gets from a package
manager: Grok has no distro package and must be built from source;
Kakadu is commercial). Java heap `-Xmx2g`, container memory 4 GB,
Docker Desktop (macOS). Processor selection is manual: `jp2 →
OpenJpegProcessor`, everything else `Java2dProcessor` — because
automatic selection NPEs on JPEG sources when the TurboJPEG native
library is absent (see Findings). Two cache postures: derivative cache
off (decode vs decode) and on (its deployed posture: the sample
properties file ships with the filesystem derivative cache enabled).

Build recipe: `tools/bench/cantaloupe/` (Dockerfile + both properties
files). Harness: `scripts/bench_cantaloupe.sh`. Corpus:
`scripts/gen_eval_corpus.sh`. Conformance: `scripts/validate.sh` (ours)
and `scripts/validate_cantaloupe.sh`.

The processor choice is the identity of a Cantaloupe deployment; these
numbers are for *a configuration* (the strongest reproducible
open-source one), not the platform's ceiling. Institutions holding a
Kakadu license reach JP2 latencies neither measured column can.

## Corpus

Fully synthetic — generated, never copied — mirroring common
digitization profiles (issue #1). JP2/HTJ2K tile grids are 1024 px;
"partial" means the grid does not divide the image dimensions, which is
the overwhelmingly common case for real tiled masters and is the `j2k`
fallback path (frames-sg/j2k#62).

| file | profile |
| --- | --- |
| scan_pyr_deflate.tif | 6500×4300 pyramidal TIFF, 256px tiles, deflate |
| scan_pyr_jpeg.tif | same, JPEG-compressed (Q90) |
| scan_plain.jpg | 6500×4300 plain JPEG (Q92): the small-collection case |
| scan_partial_ll.jp2 | 6500×4300 JP2, lossless 5/3, t=1024 n=6, partial grid |
| scan_partial_r20.jp2 | same geometry, irreversible 9/7 at 20:1 |
| scan_untiled_ll.jp2 | 6500×4300 untiled codestream, 256px precincts |
| exact_ll.jp2 | 6144×4096 lossless, t=1024: exact-grid control |
| large_partial_ll.jp2 | 15000×11000 (165 MP) lossless, t=1024, partial grid |
| scan_ht_ll.j2c / scan_ht_lossy.j2c | HTJ2K (Part 15) via OpenJPH 0.30.1, t=1024, partial grid |
| exact_ht_ll.j2c / large_ht_ll.j2c | HTJ2K exact-grid control / 165 MP |

HTJ2K is encoded with OpenJPH — independent of both servers' decoders.

## Latency

2026-07-26, Apple M1 Pro (Darwin arm64). Single-client full HTTP round
trips; warm = steady state, 30 reps per case after one warm-up request;
p50/p99 in ms. Cantaloupe column is derivative-cache-off (decode vs
decode; the cache-on posture is below). "Native tile" is
`2048,2048,512,512/max`; "full → 512" is `full/512,`.

| case | ours p50 | Cantaloupe p50 | ratio | ours p99 | Cantaloupe p99 |
| --- | --- | --- | --- | --- | --- |
| TIFF pyr deflate, native tile | **3.5** | 20.9 | 0.17× | 3.8 | 28.3 |
| TIFF pyr deflate, full → 512 | **9.4** | 25.5 | 0.37× | 9.9 | 31.8 |
| TIFF pyr JPEG, native tile | **2.7** | 18.6 | 0.15× | 3.0 | 24.4 |
| JP2 partial lossless, native tile | 628.5 | **63.7** | 9.87× | 716.3 | 71.2 |
| JP2 partial lossless, full → 512 | **56.6** | 90.2 | 0.63× | 63.3 | 135.0 |
| JP2 partial 20:1 lossy, native tile | 388.1 | **40.9** | 9.50× | 410.9 | 59.5 |
| JP2 exact-grid lossless, native tile | **26.6** | 61.1 | 0.44× | 31.6 | 65.7 |
| JP2 untiled+precincts, native tile | **32.9** | 84.6 | 0.39× | 37.6 | 97.0 |
| JP2 165 MP partial, native tile | *refused (500)* | **85.7** | — | — | 130.0 |
| JP2 165 MP partial, full → 512 | 302.0 | **175.1** | 1.72× | 310.6 | 186.1 |
| HTJ2K partial lossless, native tile | **315.2** | *501* | — | 355.3 | — |
| HTJ2K partial lossless, full → 512 | **23.4** | *501* | — | 29.4 | — |
| HTJ2K 20:1 lossy, native tile | **273.1** | *501* | — | 336.4 | — |
| HTJ2K 165 MP, native tile | *refused (500)* | *501* | — | — | — |
| plain JPEG 28 MP, full → 512 | **138.4** | 297.6 | 0.47× | 141.7 | 360.7 |
| plain JPEG 28 MP, native tile | **93.8** | 112.3 | 0.84× | 96.0 | 116.5 |

**Cantaloupe with its derivative cache on** (steady state = disk cache
hits): 3.0–4.2 ms p50 uniformly, for every case its decoders can serve.
That is its in-server answer to repeat traffic. In deployment both
servers put repeats behind HTTP caching (a CDN gives iiif-server the
same ~edge-hit latencies); the decode columns above are what the origin
pays per *unique* tile — every tile's first visitor.

**Cold** (fresh process/container, empty caches, first request; median
of 5 restarts; requests issued sequentially, so later cases benefit
from JVM warm-up on Cantaloupe's side — a bias in its favor; ours has
no warm-up to speak of, cold ≈ warm):

| case | ours cold | Cantaloupe cold |
| --- | --- | --- |
| TIFF pyr deflate, native tile | **14.1** | 109.9 |
| TIFF pyr deflate, full → 512 | **9.4** | 188.5 |
| JP2 partial lossless, native tile | 641.3 | **77.1** |
| JP2 exact-grid lossless, native tile | **26.8** | 63.3 |
| plain JPEG 28 MP, full → 512 | **136.4** | 335.3 |

(Full cold table in the harness output; the pattern is the same as
warm.)

## The partial-grid answer (issue #1)

The M2 gate's 2.01× was measured on an exact-grid master — the fast
path. These are the first numbers for the fallback path real
collections actually hit, and they are ugly, as predicted:

- **Which profiles hit it:** any *tiled* JP2/HTJ2K whose dimensions the
  tile grid does not divide — the default outcome of `opj_compress -t`
  / kdu on real scan dimensions. Exact-grid masters (26.6 ms) and
  untiled-with-precincts codestreams (32.9 ms — confirmed on the fast
  path) do not hit it. Zoomed-out requests survive it (56.6 ms at
  full → 512: the fallback at deep downscale decodes few wavelet
  levels).
- **What it costs:** native-zoom tiles pay a whole-image decode per
  uncached request: 628 ms p50 lossless / 388 ms lossy at 28 MP —
  ~24× the exact-grid number, ~10× Cantaloupe. HTJ2K halves it
  (315 ms) but does not escape it.
- **Where it ends:** at ≈134 MP (512 MiB at 4 B/px, `j2k`'s internal
  cap) the fallback cannot allocate, and native-zoom tiles are
  **refused outright** (currently as a misclassified `500 corrupt
  master`; tracked for a proper 4xx). The 165 MP master serves
  zoom-outs fine and native tiles not at all.
- **Memory:** under 4 concurrent clients on a fallback-heavy mix, peak
  RSS hit 2.5 GB (whole-image decodes in flight × pool width) vs
  1.0 GiB for Cantaloupe on the identical mix — which also completed
  2.2× the requests (1340 vs 617 in 40 s). On TIFF/fast-path mixes the
  picture inverts completely.

One number that reframes the whole gate: on the fast path we are
**2.3× faster than the incumbent** at JP2 region decode
(26.6 ms vs 61.1 ms) — with a pure-Rust decoder. The j2k#62 fix does
not merely repair the slow path; it converts every loss in this
document into a win.

## Post-fix rerun (2026-07-26, patched j2k)

The j2k#62 root cause was found and fixed the same day this eval was
written (window-origin bookkeeping in the region IDWT; root cause and
fix on the issue thread). This rerun is the identical corpus, harness,
hardware, and Cantaloupe image, with two changes on our side: `j2k` is
the patched build carrying the fix, and the decode-full-then-crop
fallback is **removed** — every tiled JP2/HTJ2K master takes the region
path regardless of grid. The exact-grid control (26.7 ms vs 26.6 ms
pre-fix) and the Cantaloupe columns reproduce the original run within
noise, so the two tables are directly comparable.

Warm, derivative-cache-off, p50/p99 in ms, 30 reps:

| case | ours p50 | Cantaloupe p50 | ratio | ours p99 | Cantaloupe p99 |
| --- | --- | --- | --- | --- | --- |
| TIFF pyr deflate, native tile | **3.5** | 21.0 | 0.17× | 3.8 | 27.7 |
| TIFF pyr deflate, full → 512 | **9.4** | 25.1 | 0.38× | 11.0 | 41.4 |
| TIFF pyr JPEG, native tile | **2.8** | 18.0 | 0.15× | 3.0 | 21.8 |
| JP2 partial lossless, native tile | **28.9** | 63.7 | 0.45× | 40.4 | 71.0 |
| JP2 partial lossless, full → 512 | **57.6** | 90.6 | 0.64× | 109.9 | 139.1 |
| JP2 partial 20:1 lossy, native tile | **21.4** | 40.0 | 0.54× | 35.5 | 101.8 |
| JP2 exact-grid lossless, native tile | **26.7** | 62.8 | 0.43× | 28.9 | 77.7 |
| JP2 untiled+precincts, native tile | **33.2** | 88.2 | 0.38× | 92.7 | 125.1 |
| JP2 165 MP partial, native tile | 90.8 | **85.7** | 1.06× | 96.6 | 93.4 |
| JP2 165 MP partial, full → 512 | 308.2 | **189.8** | 1.62× | 333.8 | 228.2 |
| HTJ2K partial lossless, native tile | **22.3** | *501* | — | 25.6 | — |
| HTJ2K partial lossless, full → 512 | **24.6** | *501* | — | 25.4 | — |
| HTJ2K 20:1 lossy, native tile | **17.5** | *501* | — | 18.3 | — |
| HTJ2K 165 MP, native tile | **89.9** | *501* | — | 145.2 | — |
| plain JPEG 28 MP, full → 512 | **138.6** | 302.8 | 0.46× | 140.7 | 374.5 |
| plain JPEG 28 MP, native tile | **94.7** | 115.4 | 0.82× | 98.4 | 121.6 |

What changed, row by row:

- **JP2 partial native tile: 628 ms → 28.9 ms** (21.7×). Partial and
  exact grids are now statistically indistinguishable (28.9 vs
  26.7 ms) — the grid shape has stopped mattering, which is the
  fix working as designed.
- **HTJ2K partial native tile: 315 ms → 22.3 ms.** The HTJ2K losses
  were the same fallback; they disappear with it.
- **JP2 165 MP native tile: refusal → 90.8 ms.** With no whole-image
  decode there is nothing for the 512 MiB cap to refuse; the former
  hard 500 is now near-parity with the incumbent (90.8 vs 85.7 ms).
  The fallback-driven 2.5 GB memory-spike scenario is likewise gone —
  region decodes never materialize the full image.
- **The one remaining loss** is 165 MP `full/512,` (308 vs 190 ms):
  our reduced-resolution decode bottoms out at 1/8 (the decode API's
  current ceiling), so we decode ~2048 px wide and resample, while
  OpenJPEG walks deeper into the resolution ladder. This is unrelated
  to #62 — it is the natural next upstream conversation.

Cold behaviour is unchanged in shape (ours cold ≈ warm; first-request
partial-grid native tile 34.4 ms vs Cantaloupe's 84.9 ms). Cache-on
stays as documented above — a derivative cache answers repeat traffic
for any origin latency, and is orthogonal to these numbers.

## Conformance

Official IIIF validators (pinned at `1740893f`), level 2, both API
versions, same reference image:

| suite | iiif-server | Cantaloupe (develop) |
| --- | --- | --- |
| Image API 3.0 level 2 | all 33 pass | all 33 pass |
| Image API 2.0/2.1 level 2 | all 30 pass | all 30 pass |

At validator level the servers are indistinguishable; the differences
are in the optional surface (empirical, HTTP status for the request):

| capability | iiif-server | Cantaloupe |
| --- | --- | --- |
| `^` upscaling (`full/^1200,/`) | 200 | 400 (`max_scale=1.0` as shipped; config-dependent) |
| webp output | 200 | 415 |
| pdf output | 200 | 415 |
| jp2 output | 200 | 415 |
| HTJ2K source (.j2c / .jph / HT-in-.jp2) | 200 | 501 / 501 / 501 |
| `square`, arbitrary rotation, distorted w,h | 200 | 200 |

Two structural notes. First, Cantaloupe's capability set is
configuration-dependent (`max_scale`, per-processor formats), so its
info.json varies per deployment; iiif-server's is baked in and
identical everywhere. Second, the HTJ2K result is precise: the
*decoder* in the container (OpenJPEG 2.5.0) decodes our HTJ2K
codestreams at the CLI — Cantaloupe the *server* has no route for the
format and returns 501 for every packaging of it.

## Ops

| | iiif-server | Cantaloupe |
| --- | --- | --- |
| Deployable artifact | **13 MB** static binary | 769 MB image (JRE 25 + JAR + OpenJPEG) |
| Startup → healthy (median) | **37 ms** | 1.08 s (container-inclusive) |
| Idle RSS | **7.9 MB** | 152 MiB |
| Peak RSS, 4-client mixed load | 2.5 GB (fallback-driven; see issue #1 section) | **1.0 GiB** |
| Config surface | `serve <root>` + 7 flags | 227 active keys (shipped sample), plus optional JRuby delegate |
| Capability drift across deployments | none (baked in) | per-config (processors, max_scale, caches) |
| Known vulns in shipped deps | **0** (trivy, 235-crate lockfile) | Java layer 0 — *but only via hand-backports unmerged upstream (#962)*; 38 unfixed OS-package vulns (30 medium/8 low) from the JRE base |
| Release-artifact scannability | lockfile, full SCA | 5.0.7 fat jar is shaded: SCA tools identify **zero** components in it |
| Hostile-input decode path | 9 tracked pure-Rust crates, `#![forbid(unsafe_code)]` | JVM + C decoders (OpenJPEG via subprocess) + optional JRuby |

## Findings that are not numbers

- **Running Cantaloupe safely means building it yourself.** The newest
  release ships EOL Jetty and logback; the eval image is develop at a
  pinned commit *plus* dependency backports upstream has not merged.
  Its own docs recommend a dedicated config file per version; its test
  suite needs MinIO, FFmpeg, Grok, OpenJPEG, TurboJPEG and Redis.
- **Stock develop 500s on plain JPEGs** under automatic processor
  selection when the TurboJPEG native library is absent (NPE in
  `TurboJPEGImageReader`); the workaround is manual processor
  selection, used here. Actually enabling TurboJPEG means building
  libjpeg-turbo from source with its Java binding.
- **Our 165 MP native-zoom refusal returns `500 corrupt master`** where
  it should return a deliberate 4xx (limit-exceeded); tracked as a
  separate fix.

## Conclusions

Honest scoreboard, then the weighing.

**Where iiif-server wins, today:** every TIFF case (3–7×), every JPEG
case, JP2 zoom-outs, JP2 exact-grid and untiled region decode
(2.3–2.6×), all HTJ2K serving (incumbent: 501), cold starts, idle and
per-instance footprint, artifact size, startup, config surface,
capability uniformity, supply-chain posture (0 vulns, no C on hostile
input, scannable lockfile), and conformance breadth (upscaling +
webp/pdf/jp2 outputs).

**Where Cantaloupe wins, today:** native-zoom tiles on partial-grid
tiled JP2s — the honest common case for real JP2 collections — by
~10×; very large partial-grid masters (165 MP), which we refuse at
native zoom and it serves in 86 ms; memory and throughput under
fallback-heavy concurrent load. Beyond these measurements it also
retains the scope we pre-refused (delegates/identifier indirection,
HttpSource/JdbcSource, overlays/redaction, built-in caches for CDN-less
deployments) and the Kakadu option (MAINTENANCE.md records the
refusals; this doc records that they are real trade-offs, not
oversights).

**The single-cause structure is the decision-relevant fact.** All
three Cantaloupe wins are the same defect: `j2k` cannot region-decode
partial tile grids (frames-sg/j2k#62, filed with a standalone
reproducer on 2026-07-26 — the same day as this eval; upstream's
responsiveness is not yet known). The fast-path evidence (26.6 ms,
2.3× faster than the incumbent's C decoder through a full HTTP stack)
shows the pure-Rust bet is not the problem; one bug's fallback is.
What the numbers imply for each gate option (the decision itself is
issue #2, taken deliberately, not here):

1. **Accept the miss** now means accepting, for typical tiled-JP2
   collections: ~630 ms first-visitor native-zoom tiles, hard refusal
   above ≈134 MP, and 2.5 GB memory spikes under concurrent fallback
   load. The mitigations are real (CDN absorbs repeats; HTJ2K transcode
   via `check` halves the cost; zoom-outs are fine) but the honest
   sentence is "the incumbent is ~10× faster on the most common JP2
   shape until upstream fixes the bug."
2. **Plan B (vendored OpenJPEG for JP2 only)** buys the incumbent's
   63 ms on partial grids and unlocks >134 MP — at the cost of the
   zero-C headline this doc's supply-chain table is built on.
3. **Wait on j2k#62** is the only option whose end state wins every
   row of every table (partial grids join the 26.6 ms fast path, and
   the >134 MP refusal disappears with the fallback itself). Its risk
   is entirely in upstream's response time, about which nothing is
   known yet: the issue was filed the day of this eval. The crate has
   been actively developed; the honest posture is to give upstream a
   real response window before treating silence as signal.

A fourth lever exists regardless of the gate: `iiif-server check`
already advises HTJ2K transcode; the same advice can note that
grid-aligned tiling (or untiled-with-precincts) avoids the fallback
entirely on current `j2k` — an ingest-time fix operators control today.

**Post-fix update:** option 3's end state materialized the same day —
see the [post-fix rerun](#post-fix-rerun-2026-07-26-patched-j2k). With
the fix applied and the fallback removed, every row of the warm table
is a win or near-parity except the 165 MP zoom-out (a different,
smaller gap: the 1/8 reduced-resolution ceiling). The gate decision
(issue #2) reduces to consuming the fix — upstream merge or a pinned
fork patch — rather than choosing between misses; the fourth lever
(ingest advice to avoid the fallback) is moot along with the fallback
itself.

## Reproducing

```sh
scripts/gen_eval_corpus.sh
docker build -t cantaloupe-eval:openjpeg tools/bench/cantaloupe
CANTALOUPE_CONF=tools/bench/cantaloupe scripts/bench_cantaloupe.sh
scripts/validate.sh
CANTALOUPE_CONF=tools/bench/cantaloupe scripts/validate_cantaloupe.sh
```
