# SPIKE 2 — `j2k` vs OpenJPEG: correctness + region-at-scale (M0)

**Question:** is the pure-Rust `j2k` crate correct against OpenJPEG on
large tiled pyramidal JP2s, fast enough on the IIIF access pattern, and
does it expose the metadata info.json needs?

**Verdict: PASS on every axis. Plan B (vendored-FFI OpenJPEG) stays
unexercised.**

## Method

`scripts/gen_spike2.sh` builds an 8192×8192 smooth deterministic master
(numpy), encodes with `opj_compress` (1024px tiles, 6 resolution levels)
as reversible 5/3 lossless and irreversible 9/7 at 20:1, plus a 4096²
lossless variant for the HTJ2K leg. Goldens are `opj_decompress` outputs
(full-res region crossing tile boundaries; whole image at reduction 2).
`crates/core/tests/spike2_j2k_vs_openjpeg.rs` (`task spike2`) decodes the
same requests through `j2k` 0.7.5 and compares per-sample.

## Results (2026-07-26, M2 Max, release build)

| Case | mean abs Δ | max abs Δ | time |
| --- | --- | --- | --- |
| lossless 512² region @ full res | 0 | **0 (bit-exact)** | 77 ms |
| lossy 512² region @ full res | 0.024 | 2 | 62 ms |
| lossless full image @ quarter res (2048² out) | 0 | **0 (bit-exact)** | 386 ms |
| lossy full image @ quarter res | 0.023 | 1 | 176 ms |
| **HTJ2K** (recoded from classic, same region) | 0 | **0 (bit-exact)** | **28 ms** |

`opj_decompress` on the same full-res region: ~170 ms wall for the whole
process (startup + decode + PPM write) — not a controlled comparison, but
`j2k` is in the same performance class or better, and the HTJ2K path is
~2.7× faster than classic on identical pixels. The real gate (p50 ≤ 1.5×
libvips, p99 ≤ 2×) runs at M2 on fixed hardware.

## The spec's open sub-questions, answered

- **Decomposition-level metadata:** exposed directly —
  `Info::resolution_levels` (reports 6 for `opj_compress -n 6`) and
  `Info::tile_layout` (1024×1024 confirmed). **No hand-rolled SIZ/COD
  parse needed** for info.json `sizes`/`tiles`.
- **Rayon pinning:** `set_cpu_decode_parallelism(CpuDecodeParallelism::
  Serial)` works and costs little on this workload (80 ms vs 78 ms) —
  the engine's worker pool owns concurrency, exactly as the architecture
  wants.
- **Downscale ladder:** the scaled-decode API caps at 1/8
  (`Downscale::Eighth`). Deeper zoom-outs decode at 1/8 and resample —
  cheap, but worth remembering when deriving `sizes`.

## Findings / limits (recorded, none blocking)

- `recode_j2k_to_htj2k_lossless` full-image recode of the 8192² master
  exceeds the crate's 512 MiB host-allocation cap, and its built-in
  round-trip validation rejects ≥4096² images (`ImageTooLarge`) — use
  `J2kEncodeValidation::External` and validate ourselves. Recode is a
  `check`-subcommand convenience, never the serving path, so neither cap
  matters at runtime.
- Input model is `&[u8]` as expected: mmap for local files; the bounded
  source-chunk cache remains the object-store answer (M4, as designed).
