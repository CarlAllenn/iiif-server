//! The codec seam across source formats: the same deterministic pattern
//! served from pyramidal TIFF, lossless JP2, plain JPEG, and plain PNG —
//! all through `open_master` and the shared pipeline.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test/bench code: a panic here is the failure signal, not a crash path"
)]

use std::{fs::File, io::Cursor, path::PathBuf};

use iiif_core::{
    codec::open_master, eval::evaluate, grammar::ImageRequest, info::Limits, pipeline,
};

const LIMITS: Limits = Limits {
    width: 8192,
    height: 8192,
    area: 67_108_864,
};

fn fixture(name: &str) -> File {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    File::open(path).expect("fixture present")
}

/// The fixture pattern: r = x % 256, g = y % 256, b = block marker.
fn expected_pixel(x: u32, y: u32) -> [u8; 3] {
    let channel = |v: u32| u8::try_from(v % 256).expect("mod 256 fits u8");
    [
        channel(x),
        channel(y),
        channel((x / 256) * 64 + (y / 256) * 32),
    ]
}

/// Serve a crop as PNG through the full pipeline and return decoded RGB.
fn crop_via_pipeline(name: &str, path: &str, full: (u32, u32)) -> (u32, u32, Vec<u8>) {
    let mut master = open_master(fixture(name)).expect("opens");
    assert_eq!(master.dimensions(), full, "{name} dimensions");
    let request = ImageRequest::parse(path).unwrap();
    let plan = evaluate(&request, full.0, full.1, LIMITS).unwrap();
    let bytes = pipeline::execute(master.as_mut(), &plan).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn assert_pattern(name: &str, buf: &[u8], out_w: u32, origin: (u32, u32), tolerance: u8) {
    for (rx, ry) in [(0u32, 0u32), (13, 40), (100, 60), (255, 100)] {
        let off = ((ry * out_w + rx) * 3) as usize;
        if off + 3 > buf.len() {
            continue;
        }
        let expected = expected_pixel(origin.0 + rx, origin.1 + ry);
        let got = [buf[off], buf[off + 1], buf[off + 2]];
        for channel in 0..3 {
            assert!(
                got[channel].abs_diff(expected[channel]) <= tolerance,
                "{name} pixel ({rx},{ry}) channel {channel}: got {got:?} expected {expected:?}"
            );
        }
    }
}

#[test]
fn jp2_partial_grid_master_serves_exact_pixels() {
    // 512px tiles on 1024×768: the bottom row is partial. Region decode
    // on partial grids needs the fixed j2k (frames-sg/j2k#62); this
    // pins bit-exactness through the region path.
    let (w, h, buf) = crop_via_pipeline(
        "rgb_pyramid.jp2",
        "300,200,256,256/max/0/default.png",
        (1024, 768),
    );
    assert_eq!((w, h), (256, 256));
    // Lossless 5/3: exact.
    assert_pattern("jp2", &buf, w, (300, 200), 0);
}

#[test]
fn jp2_exact_grid_master_serves_exact_pixels() {
    // 256px tiles divide 1024×768 exactly.
    let (w, h, buf) = crop_via_pipeline(
        "rgb_exact.jp2",
        "300,200,256,256/max/0/default.png",
        (1024, 768),
    );
    assert_eq!((w, h), (256, 256));
    assert_pattern("jp2-exact", &buf, w, (300, 200), 0);
}

#[test]
fn jp2_describe_exposes_resolution_ladder() {
    let master = open_master(fixture("rgb_pyramid.jp2")).expect("opens");
    let description = master.describe();
    assert_eq!(description.width, 1024);
    assert_eq!(description.tiles.len(), 1);
    assert_eq!(description.tiles[0].width, 512);
    assert_eq!(description.tiles[0].scale_factors, vec![1, 2, 4, 8]);
    assert_eq!(description.sizes.len(), 4);
    assert_eq!(description.sizes.last().unwrap().width, 1024);
}

#[test]
fn jp2_downscaled_request_uses_reduced_resolution() {
    // Full image at 256 wide: the codec should decode at 1/4, and the
    // result must still match the pattern (averaged, so tolerance).
    let (w, h, buf) = crop_via_pipeline("rgb_pyramid.jp2", "full/256,/0/default.png", (1024, 768));
    assert_eq!((w, h), (256, 192));
    // Downsampled smooth ramps stay near the midpoint sample.
    let off = ((100 * 256 + 100) * 3) as usize;
    let expected = expected_pixel(400, 400);
    assert!(buf[off].abs_diff(expected[0]) <= 8);
}

#[test]
fn plain_jpeg_master_serves() {
    let (w, h, buf) = crop_via_pipeline(
        "rgb_plain.jpg",
        "300,200,256,256/max/0/default.png",
        (1024, 768),
    );
    assert_eq!((w, h), (256, 256));
    // Q92 JPEG of a smooth-ish ramp: small tolerance, structure intact.
    assert_pattern("jpeg", &buf, w, (300, 200), 12);
}

#[test]
fn plain_png_master_serves() {
    let (w, h, buf) = crop_via_pipeline(
        "rgb_plain.png",
        "100,100,256,256/max/0/default.png",
        (512, 384),
    );
    assert_eq!((w, h), (256, 256));
    assert_pattern("png", &buf, w, (100, 100), 0);
}

#[test]
fn simple_masters_describe_honestly() {
    let master = open_master(fixture("rgb_plain.png")).expect("opens");
    let description = master.describe();
    assert!(description.tiles.is_empty(), "no fake tiling advertised");
    assert_eq!(description.sizes.len(), 1);
}

#[test]
fn unrecognized_master_is_actionable() {
    let Err(err) = open_master(Cursor::new(b"not an image at all".to_vec())) else {
        panic!("garbage must not open");
    };
    assert!(err.to_string().contains("supported"), "got: {err}");
}

#[test]
fn declared_dimension_bomb_is_rejected_before_allocating() {
    // Regression: found by fuzzing (fuzz/fuzz_targets/master_open.rs).
    // A 90-byte PNG header claiming 512×16777335 drove a 25 GB
    // allocation before any pixel arrived; the whole-decode ceiling now
    // rejects it at the header, with conversion advice.
    let Err(err) = open_master(fixture("bomb_declared_512x16777335.png")) else {
        panic!("dimension bomb must be rejected");
    };
    let message = err.to_string();
    assert!(message.contains("ceiling"), "got: {message}");
    assert!(message.contains("pyramidal"), "advice missing: {message}");
}
