# Benchmark gate vs libvips (M2)

**Gate (design spec):** p50 ≤ 1.5× libvips, p99 ≤ 2×, on a stated corpus
and stated hardware.

Run: `scripts/bench_libvips.sh` (needs the spike fixtures:
`scripts/gen_spike1.sh`, `scripts/gen_spike2.sh`).

## Method, and one honesty note

Our numbers are full HTTP round trips against the release server
(request parse → evaluate → decode → resample → encode → response). The
libvips numbers are `vips` CLI invocations on the same masters for the
same regions — **minus the measured CLI startup floor** (~89 ms of mise
shim + dynamic linking on this machine), because our server pays startup
once and the CLI pays it per invocation. Comparing without that
subtraction would flatter us by 30×; the subtracted numbers are the
honest decode-vs-decode comparison, and they are what the table reports.

## Results (2026-07-26, Apple M1 Pro, 30 reps per case)

| Case | ours p50 | libvips p50 | ratio | ours p99 | libvips p99 | ratio | gate |
| --- | --- | --- | --- | --- | --- | --- | --- |
| pyramidal TIFF, 512² tile @ native | 2.61 ms | 4.84 ms | **0.54×** | 2.88 ms | 11.31 ms | **0.25×** | PASS |
| pyramidal TIFF, full image → 512 wide | 2.35 ms | 10.75 ms | **0.22×** | 2.48 ms | 11.00 ms | **0.23×** | PASS |
| JP2 8192², 512² region @ native | 46.70 ms | 23.20 ms | 2.01× | 52.65 ms | 24.24 ms | 2.17× | **FAIL** |

TIFF — the dominant real-world case — is **2–4× faster than libvips**.
JP2 region decode misses the gate.

## The JP2 gap, characterized

Profiling the failing case (`j2k` 0.7.5, same 8192² lossless master,
512² region):

| Configuration | p50 |
| --- | --- |
| parse/header only | ~0 ms (free — decoder caching would buy nothing) |
| decode, codec-internal parallelism off | 66–77 ms |
| decode, codec-internal parallelism on | 39–47 ms |
| libvips/OpenJPEG equivalent | 23 ms |

So the cost is the wavelet decode itself, not our plumbing.

**Fixed in this pass:** parallelism is now chosen from live pool
pressure rather than pinned off. Measured crossover on the same machine:

| Concurrency | serial | codec-parallel |
| --- | --- | --- |
| 1 client | 15.3 ops/s | **25.6 ops/s** |
| 4 clients | 52.5 ops/s | **61.1 ops/s** |
| 8 clients (saturated) | **81.2 ops/s** | 67.9 ops/s |

An idle pool lends its cores to the codec (1.7× better latency); a
saturated pool keeps them (16% better throughput, no oversubscription).
That took JP2 from 3.21× to 2.01× — real, but still outside the gate.

## Open decision (not taken unilaterally)

The design spec's rule is: *miss → Plan B evaluation scoped to the
failing codec only (vendored-FFI OpenJPEG for JP2)*. Taking Plan B would
buy roughly 2× on JP2 region decode and **cost the zero-C headline**,
which is a product-positioning decision, not an engineering one. The
options, for the record:

1. **Accept 2× on JP2** — 47 ms per uncached tile is interactive, the
   CDN absorbs repeats, and the pure-Rust/zero-C claim stays intact.
   Note also that HTJ2K masters decode ~2.7× faster than classic JP2
   (SPIKE 2), so the `check` subcommand's transcode advice is a real
   mitigation available to operators today.
2. **Plan B for JP2 only** — vendored OpenJPEG behind the existing codec
   trait (a contained swap; the differential goldens already exist to
   verify it), at the cost of C in the decode path.
3. **Wait on `j2k`** — it is young (0.7.x) and improving; the gap may
   close upstream. The codec seam means this costs nothing to wait on.

Recommendation: **option 1 for now**, revisit at the launch milestone
with real-corpus numbers rather than synthetic ones.
