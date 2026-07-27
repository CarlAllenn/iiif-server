//! M0 object-store mini-spike: range-read latency profile against an
//! S3-compatible endpoint (`MinIO` locally; Hetzner later is an env-var
//! swap). Produces the metadata-cache sizing numbers for M4.
//!
//! Environment: `SPIKE_ENDPOINT` (e.g. `http://127.0.0.1:9000`),
//! `SPIKE_BUCKET` (must exist), `AWS_ACCESS_KEY_ID` /
//! `AWS_SECRET_ACCESS_KEY`, and `SPIKE_OBJECT` (local file to
//! upload and measure).
//!
//! Run: `cargo run --release -p iiif-sources --example objstore_spike`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "diagnostic spike harness: prints findings, panics are failures"
)]

use std::time::Instant;

use object_store::{
    GetOptions, GetRange, ObjectStore, ObjectStoreExt, PutPayload, aws::AmazonS3Builder,
    path::Path as ObjectPath,
};

/// Nearest-rank percentile; integer arithmetic keeps it exact.
fn percentile(sorted: &[f64], pct: usize) -> f64 {
    let rank = (pct * (sorted.len() - 1) + 50) / 100;
    sorted[rank.min(sorted.len() - 1)]
}

async fn timed_ranges(
    store: &dyn ObjectStore,
    path: &ObjectPath,
    size: u64,
    object_len: u64,
    reps: usize,
) -> (f64, f64) {
    let mut samples = Vec::with_capacity(reps);
    for rep in 0..reps {
        // Scatter offsets deterministically across the object.
        let offset = (rep as u64 * 7_919_993) % object_len.saturating_sub(size).max(1);
        let options = GetOptions {
            range: Some(GetRange::Bounded(offset..offset + size)),
            ..GetOptions::default()
        };
        let started = Instant::now();
        let result = store.get_opts(path, options).await.expect("range read");
        let bytes = result.bytes().await.expect("body");
        assert_eq!(bytes.len() as u64, size);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    (percentile(&samples, 50), percentile(&samples, 95))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Explicit provider choice (ring), not a silent default — the
    // outbound-TLS crypto question is an M4 decision on the record.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install crypto provider once");
    let endpoint = std::env::var("SPIKE_ENDPOINT").expect("SPIKE_ENDPOINT");
    let bucket = std::env::var("SPIKE_BUCKET").expect("SPIKE_BUCKET");
    let object_file = std::env::var("SPIKE_OBJECT").expect("SPIKE_OBJECT");

    let store = AmazonS3Builder::from_env()
        .with_endpoint(&endpoint)
        .with_bucket_name(&bucket)
        .with_allow_http(true)
        .with_region("us-east-1")
        .build()
        .expect("S3 store");
    let path = ObjectPath::from("spike-master.jp2");

    let data = std::fs::read(&object_file).expect("SPIKE_OBJECT readable");
    let object_len = data.len() as u64;
    let started = Instant::now();
    store
        .put(&path, PutPayload::from(data))
        .await
        .expect("upload");
    println!(
        "uploaded {object_len} bytes in {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    // HEAD latency — what an uncached info.json request pays first.
    let mut heads = Vec::new();
    for _ in 0..40 {
        let started = Instant::now();
        store.head(&path).await.expect("head");
        heads.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    heads.sort_by(f64::total_cmp);
    println!(
        "HEAD: p50 {:.2} ms, p95 {:.2} ms",
        percentile(&heads, 50),
        percentile(&heads, 95)
    );

    // Ranged GETs at the sizes the engine actually issues: header sniffs
    // (4 KiB), IFD/tile-index reads (64 KiB), tile payloads (1 MiB).
    for (label, size) in [
        ("4 KiB", 4_096u64),
        ("64 KiB", 65_536),
        ("1 MiB", 1_048_576),
    ] {
        let (p50, p95) = timed_ranges(&store, &path, size, object_len, 40).await;
        println!("range {label}: p50 {p50:.2} ms, p95 {p95:.2} ms");
    }

    // The uncached-open simulation: the sequential round trips a cold
    // TIFF/JP2 open costs before the first pixel moves (header, then
    // index, then first tile) — the number the metadata cache exists to
    // amortize.
    let mut opens = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        for (offset, len) in [(0u64, 4_096u64), (65_536, 65_536), (1_048_576, 1_048_576)] {
            let options = GetOptions {
                range: Some(GetRange::Bounded(offset..offset + len)),
                ..GetOptions::default()
            };
            let result = store.get_opts(&path, options).await.expect("read");
            drop(result.bytes().await.expect("body"));
        }
        opens.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    opens.sort_by(f64::total_cmp);
    println!(
        "cold-open simulation (3 sequential reads): p50 {:.2} ms, p95 {:.2} ms",
        percentile(&opens, 50),
        percentile(&opens, 95)
    );

    // Coalescing: many scattered small ranges in one get_ranges call.
    let ranges: Vec<std::ops::Range<u64>> = (0..16)
        .map(|i| {
            let offset = (i * 524_288) % object_len.saturating_sub(8_192);
            offset..offset + 8_192
        })
        .collect();
    let started = Instant::now();
    let chunks = store.get_ranges(&path, &ranges).await.expect("get_ranges");
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "get_ranges: 16×8 KiB scattered in {:.2} ms ({} chunks returned)",
        elapsed,
        chunks.len()
    );
}
