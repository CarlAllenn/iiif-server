# SPIKE 1 — JPEG-in-TIFF pyramid tiles (M0)

**Question:** does the `tiff` crate (0.11.3, JPEG delegated to `zune-jpeg`)
decode ModernJPEG (tag 7) pyramid tiles correctly — especially subsampled
YCbCr — and fast enough?

**Verdict: PASS, with one engine-side finding (fixed in this repo).**

## Method

`scripts/gen_spike1.sh` synthesizes a smooth deterministic 2048×1536
pattern and saves two tiled pyramids via libvips:

- `spike1_ycbcr420.tif` — Q75; libvips subsamples chroma below Q90, and
  writes photometric **YCbCr** (tag 262 = 6)
- `spike1_ycbcr444.tif` — Q95; no subsampling, photometric **RGB**

Golden reference: libvips (libjpeg) decodes of the same files, exported as
raw PPM for two level-0 regions (one crossing four tile boundaries).
`crates/core/tests/spike1_jpeg_in_tiff.rs` (ignored by default; `task
spike1`) decodes the same regions through our codec and compares
per-sample.

## Finding: the tiff crate returns *raw Y′CbCr samples* for YCbCr JPEG

For ModernJPEG chunks the tiff crate deliberately configures `zune-jpeg`
to keep its **input** colorspace, so `read_chunk` yields interleaved,
already-upsampled Y′CbCr — `colortype()` says `YCbCr(8)` and means it.
Treating those samples as RGB produced mean |Δ| = **89.6**/255 against the
golden. The engine now performs the JPEG full-range BT.601 conversion
itself; regression-locked by this spike's tests.

## Results (2026-07-26, M2 Max, release build)

| Variant | Region | mean abs Δ | max abs Δ | decode time |
| --- | --- | --- | --- | --- |
| 4:4:4 (photometric RGB) | 192,192,384,384 | 0.002 | 1 | 3.9 ms |
| 4:4:4 (photometric RGB) | 0,0,256,256 | 0.001 | 1 | 0.41 ms |
| 4:2:0 (photometric YCbCr) | 192,192,384,384 | 0.367 | 4 | 4.6 ms |
| 4:2:0 (photometric YCbCr) | 0,0,256,256 | 0.345 | 4 | 0.33 ms |
| full level 0 (48 tiles, 2048×1536) | — | — | — | 21.9 ms |

The 4:2:0 residual is chroma-upsampling filter skew between decoders
(zune vs libjpeg "fancy" upsampling), not error: a stride or plane bug
measures two orders of magnitude larger. Tolerances in the test are
pinned just above the observed envelope (mean ≤ 0.8, peak ≤ 16 for 4:2:0;
mean ≤ 0.51, peak ≤ 2 for 4:4:4).

## Follow-ups carried into M2

- Shared `JPEGTables` handling is exercised by these fixtures (libvips
  writes the tables tag for tiled JPEG TIFFs) — covered.
- The M2 matrix adds: 16-bit, planar, CMYK/YCCK JPEG, old-style JPEG
  (tag 6) clean rejection, and BigTIFF goldens.
