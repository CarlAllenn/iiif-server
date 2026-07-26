# Allocator bench — musl malloc vs mimalloc (M0)

**Question:** does musl's malloc serialize the concurrent
decode→resize→encode workload badly enough to justify shipping mimalloc
(C, but trusted-compute C that never parses hostile bytes)?

**Verdict: yes — mimalloc ships as the default allocator.** The M2
benchmark gate measures our pipeline, not musl's malloc, exactly as the
design spec demanded.

## Method

`scripts/spike_alloc.sh` runs `crates/server/examples/alloc_bench.rs`
inside `rust:1.97-alpine` (musl-native, the shipping environment): 8
threads × 40 iterations of open → region decode (JPEG-in-TIFF 4:2:0
pyramid) → resize → transform → encode, with a mixed request pattern.
Same binary twice: default allocator, then `--features mimalloc`.

## Results (2026-07-26, Docker linux/arm64 on M2 Max, 3 runs each)

| Allocator | ops/s (runs 1→3) |
| --- | --- |
| musl malloc | 213 · 226 · 260 |
| mimalloc | 318 · 451 · 522 |

mimalloc is **1.5–2.0× faster** and still climbing across runs (musl
plateaus early — the contention signature). The `mimalloc` feature is
default-on for the server binary; `--no-default-features` builds pure
allocator-free-C if anyone needs the purity build.

The precise headline, unchanged: **zero C parses untrusted input.**
mimalloc computes over our own allocations only.
