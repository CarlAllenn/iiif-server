# Object-store mini-spike — range-read profile vs MinIO (M0)

**Question:** what does the S3 range-read pattern cost through
`object_store`, and what should the M4 source-metadata cache amortize?

Run: `task spike:objstore` (Docker MinIO). Pointing the same harness at
Hetzner Object Storage is an env-var swap (`SPIKE_ENDPOINT`,
`SPIKE_BUCKET`, credentials) — real-network numbers deferred to M4 by
decision (2026-07-26).

## Results (localhost MinIO, 17 MB JP2 master, 2026-07-26)

| Operation | p50 | p95 |
| --- | --- | --- |
| HEAD | 0.45 ms | 0.57 ms |
| ranged GET, 4 KiB | 1.27 ms | 2.18 ms |
| ranged GET, 64 KiB | 1.73 ms | 2.23 ms |
| ranged GET, 1 MiB | 2.55 ms | 3.94 ms |
| cold-open simulation (3 sequential reads) | 3.81 ms | 4.55 ms |
| `get_ranges` 16×8 KiB scattered | 10.3 ms total | — |

## What transfers to the real network

Localhost hides propagation latency, so read the *shape*, not the values:

- **Cold open = 3 sequential round trips** (header → index → first tile)
  before the first pixel moves. At a realistic 15–30 ms RTT to an object
  store, that is 45–90 ms of pure latency per uncached request — the
  bounded in-memory source-metadata cache (TIFF IFDs/JP2 headers) exists
  precisely to reduce this to one round trip (the tile itself). Cache
  entries are tens of KB per master; sizing is generous at default.
- Range size barely moves the needle vs round-trip count (4 KiB and
  1 MiB differ by ~1.3 ms locally; bandwidth is not the constraint) —
  **coalescing wins over precision.** `get_ranges` handles scattered
  small reads in one call.
- HEAD is cheap and gives ETag + length — the M5 conditional-request
  path can afford it when the metadata cache is cold.

## Decision flagged for M4 (outbound TLS crypto provider)

`object_store`'s packaged `aws` feature pulls rustls with the **aws-lc**
provider (C crypto); the alternative packaged provider is **ring**
(C/assembly cores as well). The spike deliberately builds with
`rustls-no-provider` and installs **ring** explicitly, so the choice is
visible in code, not a transitive default. The doctrine question for the
record: outbound TLS record processing is byte parsing of
semi-trusted-infrastructure traffic — decide at M4 whether that C is
classified like mimalloc (trusted compute) with the headline unchanged
(*zero C parses client input*), or whether a pure-Rust provider is
mature enough by then. Local MinIO runs over plain HTTP; nothing tonight
depends on the answer.
