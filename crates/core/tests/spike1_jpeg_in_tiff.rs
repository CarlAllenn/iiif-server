//! SPIKE 1 (M0): JPEG-in-TIFF pyramid correctness, especially subsampled
//! YCbCr, against libvips/libjpeg golden decodes of the same files.
//!
//! Requires generated fixtures: `task spike1` (or `scripts/gen_spike1.sh`).
//! Ignored by default so `cargo test` stays hermetic; the spike runner
//! executes with `--ignored`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test/bench code: a panic here is the failure signal, not a crash path"
)]
#![allow(
    clippy::print_stdout,
    reason = "spike harness: measured timings/deltas are the test's output"
)]

use std::{fs::File, path::PathBuf, time::Instant};

use iiif_core::{codec::TiffPyramid, image::Raster};
use num_traits::cast::ToPrimitive;

fn generated(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/generated")
        .join(name)
}

/// Minimal binary-PPM (P6, maxval 255) reader for the golden files.
fn read_ppm(name: &str) -> (u32, u32, Vec<u8>) {
    let data = std::fs::read(generated(name))
        .unwrap_or_else(|_| panic!("golden {name} missing — run `task spike1` first"));
    let mut fields = Vec::new();
    let mut pos = 0;
    while fields.len() < 4 {
        // Skip whitespace and comments.
        while pos < data.len() && (data[pos].is_ascii_whitespace()) {
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
    pos += 1; // single whitespace after maxval
    assert_eq!(fields[0], "P6");
    assert_eq!(fields[3], "255");
    let (w, h): (u32, u32) = (fields[1].parse().unwrap(), fields[2].parse().unwrap());
    (w, h, data[pos..].to_vec())
}

struct Comparison {
    mean_abs_error: f64,
    max_abs_error: u8,
}

fn compare(ours: &Raster, golden: &[u8]) -> Comparison {
    let ours = ours.data();
    assert_eq!(ours.len(), golden.len(), "pixel count mismatch");
    let mut sum = 0u64;
    let mut max = 0u8;
    for (a, b) in ours.iter().zip(golden) {
        let delta = a.abs_diff(*b);
        sum += u64::from(delta);
        max = max.max(delta);
    }
    let count = u32::try_from(ours.len()).expect("test buffers fit u32");
    Comparison {
        mean_abs_error: sum.to_f64().unwrap_or(f64::MAX) / f64::from(count),
        max_abs_error: max,
    }
}

fn check_variant(variant: &str, max_mean: f64, max_peak: u8) {
    let path = generated(&format!("spike1_{variant}.tif"));
    let mut tiff = TiffPyramid::open(
        File::open(&path).unwrap_or_else(|_| panic!("fixture missing — run `task spike1` first")),
    )
    .expect("JPEG-in-TIFF pyramid opens");
    assert_eq!(tiff.dimensions(), (2048, 1536));
    assert!(tiff.levels().len() >= 3, "pyramid expected");

    for (x, y, w, h) in [(192u32, 192u32, 384u32, 384u32), (0, 0, 256, 256)] {
        let started = Instant::now();
        let raster = tiff.decode_region(0, x, y, w, h).expect("region decodes");
        let elapsed = started.elapsed();
        let (gw, gh, golden) = read_ppm(&format!("spike1_golden_{variant}_{x}_{y}_{w}_{h}.ppm"));
        assert_eq!((raster.width(), raster.height()), (gw, gh));
        let result = compare(&raster, &golden);
        println!(
            "spike1 {variant} region {x},{y},{w},{h}: mean |Δ| = {:.3}, max |Δ| = {} \
            (decode {elapsed:?})",
            result.mean_abs_error, result.max_abs_error
        );
        assert!(
            result.mean_abs_error <= max_mean,
            "{variant}: mean error {} exceeds {max_mean}",
            result.mean_abs_error
        );
        assert!(
            result.max_abs_error <= max_peak,
            "{variant}: max error {} exceeds {max_peak}",
            result.max_abs_error
        );
    }
}

#[test]
#[ignore = "needs generated fixtures: task spike1"]
fn ycbcr_444_matches_libjpeg_golden() {
    // Same JPEG bitstream, two conformant decoders: tiny rounding skew
    // only.
    check_variant("ycbcr444", 0.51, 2);
}

#[test]
#[ignore = "needs generated fixtures: task spike1"]
fn ycbcr_420_subsampled_matches_libjpeg_golden() {
    // Chroma upsampling filters legitimately differ between decoders
    // (fancy vs linear); the tolerance covers filter skew, not decode
    // bugs — a wrong-stride or wrong-plane bug produces errors two
    // orders of magnitude larger.
    check_variant("ycbcr420", 0.8, 16);
}

#[test]
#[ignore = "needs generated fixtures: task spike1"]
fn full_level_decode_perf_sanity() {
    // Perf floor, not a benchmark: decoding the full 2048×1536 level-0
    // (48 JPEG tiles) must be comfortably interactive.
    let mut tiff =
        TiffPyramid::open(File::open(generated("spike1_ycbcr420.tif")).unwrap()).expect("opens");
    let started = Instant::now();
    let raster = tiff.decode_region(0, 0, 0, 2048, 1536).expect("decodes");
    let elapsed = started.elapsed();
    println!("spike1 full-level decode 2048×1536: {elapsed:?}");
    assert_eq!((raster.width(), raster.height()), (2048, 1536));
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "pathologically slow decode: {elapsed:?}"
    );
}
