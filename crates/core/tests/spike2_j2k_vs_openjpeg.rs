//! SPIKE 2 (M0): `j2k` correctness and region-at-scale performance against
//! `OpenJPEG` goldens, on a large tiled pyramidal JP2.
//!
//! Requires generated fixtures: `task spike2` (or `scripts/gen_spike2.sh`).
//! Ignored by default so `cargo test` stays hermetic.
//!
//! Also answers the two open sub-questions from the design spec:
//! decomposition-level metadata exposure (yes: `Info::resolution_levels`
//! and `Info::tile_layout`) and rayon pinning (`CpuDecodeParallelism`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test/bench code: a panic here is the failure signal, not a crash path"
)]
#![allow(
    clippy::print_stdout,
    reason = "spike harness: measured timings/deltas are the test's output"
)]

use std::{path::PathBuf, time::Instant};

use j2k::{CpuDecodeParallelism, Downscale, J2kDecoder, J2kScratchPool, PixelFormat, Rect};

const REGION: Rect = Rect {
    x: 3072,
    y: 2560,
    w: 512,
    h: 512,
};

fn generated(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/generated")
        .join(name)
}

fn fixture_bytes(variant: &str) -> Vec<u8> {
    std::fs::read(generated(&format!("spike2_{variant}.jp2")))
        .unwrap_or_else(|_| panic!("fixture missing — run `task spike2` first"))
}

/// Minimal binary-PPM (P6, maxval 255) reader for the golden files.
fn read_ppm(name: &str) -> (u32, u32, Vec<u8>) {
    let data = std::fs::read(generated(name))
        .unwrap_or_else(|_| panic!("golden {name} missing — run `task spike2` first"));
    let mut fields = Vec::new();
    let mut pos = 0;
    while fields.len() < 4 {
        while pos < data.len() && data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if data[pos] == b'#' {
            while data[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while pos < data.len() && !data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        fields.push(std::str::from_utf8(&data[start..pos]).unwrap().to_owned());
    }
    pos += 1;
    assert_eq!(fields[0], "P6");
    assert_eq!(fields[3], "255");
    let (w, h): (u32, u32) = (fields[1].parse().unwrap(), fields[2].parse().unwrap());
    (w, h, data[pos..].to_vec())
}

fn error_stats(ours: &[u8], golden: &[u8]) -> (f64, u8) {
    assert_eq!(ours.len(), golden.len(), "sample count mismatch");
    let mut sum = 0u64;
    let mut max = 0u8;
    for (a, b) in ours.iter().zip(golden) {
        let delta = a.abs_diff(*b);
        sum += u64::from(delta);
        max = max.max(delta);
    }
    let count = u32::try_from(ours.len()).expect("test buffers fit u32");
    let sum = u32::try_from(sum).expect("error sum fits u32 for test-sized buffers");
    (f64::from(sum) / f64::from(count), max)
}

#[test]
#[ignore = "needs generated fixtures: task spike2"]
fn metadata_exposes_pyramid_structure() {
    let bytes = fixture_bytes("lossless");
    let decoder = J2kDecoder::new(&bytes).expect("parses");
    let info = decoder.info();
    assert_eq!(info.dimensions, (8192, 8192));
    assert_eq!(info.components, 3);
    assert_eq!(info.bit_depth, 8);
    // opj_compress -n 6 requests six resolutions (five DWT
    // decompositions); j2k reports exactly that. This is the info.json
    // `sizes`/`tiles` source — no hand-rolled SIZ/COD parse needed.
    assert_eq!(info.resolution_levels, 6);
    let tiles = info.tile_layout.as_ref().expect("tiled codestream");
    assert_eq!((tiles.tile_width, tiles.tile_height), (1024, 1024));
    println!(
        "spike2 metadata: dims {:?}, {} resolution levels, {}×{} tiles",
        info.dimensions, info.resolution_levels, tiles.tile_width, tiles.tile_height
    );
}

fn decode_region(
    variant: &str,
    parallelism: CpuDecodeParallelism,
) -> (Vec<u8>, std::time::Duration) {
    let bytes = fixture_bytes(variant);
    let mut decoder = J2kDecoder::new(&bytes).expect("parses");
    decoder.set_cpu_decode_parallelism(parallelism);
    let mut pool = J2kScratchPool::new();
    let mut out = vec![0u8; 512 * 512 * 3];
    let started = Instant::now();
    decoder
        .decode_region_into(&mut pool, &mut out, 512 * 3, PixelFormat::Rgb8, REGION)
        .expect("region decodes");
    (out, started.elapsed())
}

#[test]
#[ignore = "needs generated fixtures: task spike2"]
fn lossless_region_is_bit_exact_vs_openjpeg() {
    let (ours, elapsed) = decode_region("lossless", CpuDecodeParallelism::Auto);
    let (gw, gh, golden) = read_ppm("spike2_golden_lossless_region.ppm");
    assert_eq!((gw, gh), (512, 512));
    let (mean, max) = error_stats(&ours, &golden);
    println!("spike2 lossless region: mean |Δ| = {mean:.4}, max |Δ| = {max} ({elapsed:?})");
    assert_eq!(max, 0, "reversible 5/3 must be bit-exact across decoders");
}

#[test]
#[ignore = "needs generated fixtures: task spike2"]
fn lossy_region_matches_openjpeg() {
    let (ours, elapsed) = decode_region("lossy", CpuDecodeParallelism::Auto);
    let (gw, gh, golden) = read_ppm("spike2_golden_lossy_region.ppm");
    assert_eq!((gw, gh), (512, 512));
    let (mean, max) = error_stats(&ours, &golden);
    println!("spike2 lossy region: mean |Δ| = {mean:.4}, max |Δ| = {max} ({elapsed:?})");
    // Irreversible 9/7 permits small cross-decoder float skew — never
    // structural error.
    assert!(mean <= 0.5, "mean error {mean}");
    assert!(max <= 4, "max error {max}");
}

#[test]
#[ignore = "needs generated fixtures: task spike2"]
fn region_at_scale_matches_reduced_resolution_golden() {
    for (variant, exact) in [("lossless", true), ("lossy", false)] {
        let bytes = fixture_bytes(variant);
        let mut decoder = J2kDecoder::new(&bytes).expect("parses");
        let mut pool = J2kScratchPool::new();
        let mut out = vec![0u8; 2048 * 2048 * 3];
        let started = Instant::now();
        decoder
            .decode_region_scaled_into(
                &mut pool,
                &mut out,
                2048 * 3,
                PixelFormat::Rgb8,
                Rect {
                    x: 0,
                    y: 0,
                    w: 8192,
                    h: 8192,
                },
                Downscale::Quarter,
            )
            .expect("scaled decode");
        let elapsed = started.elapsed();
        let (gw, gh, golden) = read_ppm(&format!("spike2_golden_{variant}_r2.ppm"));
        assert_eq!((gw, gh), (2048, 2048));
        let (mean, max) = error_stats(&out, &golden);
        println!(
            "spike2 {variant} quarter-scale full-image: mean |Δ| = {mean:.4}, \
            max |Δ| = {max} ({elapsed:?})"
        );
        if exact {
            assert_eq!(max, 0, "reduced-resolution reversible decode must be exact");
        } else {
            assert!(mean <= 0.5, "mean error {mean}");
            assert!(max <= 4, "max error {max}");
        }
    }
}

#[test]
#[ignore = "needs generated fixtures: task spike2"]
fn region_at_scale_perf_gate() {
    // The IIIF hot path: a tile-sized region from a huge image, at native
    // and reduced scale. Numbers are recorded in docs/spikes/; the hard
    // assertion here is only "interactive" (< 250 ms release / < 2.5 s
    // debug per op).
    let budget = if cfg!(debug_assertions) { 2.5 } else { 0.25 };
    let (_, auto_time) = decode_region("lossless", CpuDecodeParallelism::Auto);
    let (_, serial_time) = decode_region("lossless", CpuDecodeParallelism::Serial);
    println!(
        "spike2 512×512 region decode: parallel {auto_time:?}, serial (pinned for \
        worker-pool integration) {serial_time:?}"
    );
    assert!(
        auto_time.as_secs_f64() < budget,
        "parallel too slow: {auto_time:?}"
    );
    assert!(
        serial_time.as_secs_f64() < budget * 4.0,
        "serial too slow: {serial_time:?}"
    );
}

#[test]
#[ignore = "needs generated fixtures: task spike2"]
fn htj2k_recode_then_decode_matches_golden() {
    // HTJ2K arrives free with j2k: recode a classic lossless master to
    // HT (lossless), decode the same region, compare to the same OpenJPEG
    // golden — three-way agreement classic/HT/opj. The 4096² master is
    // used because full-image recode of the 8192² one exceeds j2k's
    // 512 MiB host-allocation cap (finding recorded in docs/spikes/).
    let bytes = fixture_bytes("lossless4k");
    // The built-in round-trip validation rejects images this large; this
    // test IS the external validation (it decodes the result and compares
    // to the OpenJPEG golden).
    let mut options = j2k::J2kToHtj2kOptions::default();
    options.validation = j2k::J2kEncodeValidation::External;
    let recoded = j2k::recode_j2k_to_htj2k_lossless(&bytes, options).expect("recode to HTJ2K");
    let mut decoder = J2kDecoder::new(&recoded.bytes).expect("HTJ2K parses");
    let mut pool = J2kScratchPool::new();
    let mut out = vec![0u8; 512 * 512 * 3];
    let region = Rect {
        x: 1024,
        y: 1024,
        w: 512,
        h: 512,
    };
    let started = Instant::now();
    decoder
        .decode_region_into(&mut pool, &mut out, 512 * 3, PixelFormat::Rgb8, region)
        .expect("HTJ2K region decodes");
    let elapsed = started.elapsed();
    let (_, _, golden) = read_ppm("spike2_golden_lossless4k_region.ppm");
    let (mean, max) = error_stats(&out, &golden);
    println!("spike2 HTJ2K region: mean |Δ| = {mean:.4}, max |Δ| = {max} ({elapsed:?})");
    assert_eq!(max, 0, "lossless HT recode must stay bit-exact");
}
