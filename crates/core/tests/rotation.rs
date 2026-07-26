//! Arbitrary rotation semantics: canvas growth, transparent corners for
//! PNG, white corners for JPEG, and interior pixel preservation.

use iiif_core::codec::open_master;
use iiif_core::eval::evaluate;
use iiif_core::grammar::ImageRequest;
use iiif_core::info::Limits;
use iiif_core::pipeline;
use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;

const LIMITS: Limits = Limits {
    max_width: 8192,
    max_height: 8192,
    max_area: 67_108_864,
};

fn serve(path: &str) -> Vec<u8> {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/rgb_pyramid.tif");
    let mut master = open_master(File::open(fixture).unwrap()).unwrap();
    let request = ImageRequest::parse(path).unwrap();
    let plan = evaluate(&request, 1024, 768, LIMITS).unwrap();
    pipeline::execute(master.as_mut(), &plan).unwrap()
}

#[test]
fn rotation_45_png_has_grown_canvas_and_transparent_corners() {
    let bytes = serve("0,0,256,256/max/45/default.png");
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    // 256×256 at 45°: diagonal ≈ 363.
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert!(
        info.width >= 362 && info.width <= 364,
        "width {}",
        info.width
    );
    assert_eq!(info.width, info.height);
    // Corner transparent, center opaque.
    assert_eq!(buf[3], 0, "top-left corner alpha");
    let center = ((info.height / 2) * info.width + info.width / 2) as usize * 4;
    assert_eq!(buf[center + 3], 255, "center alpha");
}

#[test]
fn rotation_45_jpeg_has_white_corners() {
    let bytes = serve("0,0,256,256/max/45/default.jpg");
    let mut decoder = zune_jpeg::JpegDecoder::new(Cursor::new(&bytes));
    let pixels = decoder.decode().unwrap();
    let (w, _) = decoder.dimensions().unwrap();
    assert!((362..=364).contains(&w), "width {w}");
    // Top-left corner is outside the rotated frame → white-ish (JPEG
    // ringing allowed).
    assert!(pixels[0] > 230, "corner should be white, got {}", pixels[0]);
}

#[test]
fn rotation_360_is_identity_geometry() {
    let bytes = serve("0,0,256,256/max/360/default.png");
    let reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let info = reader.info();
    assert_eq!((info.width, info.height), (256, 256));
}

#[test]
fn gray_rotation_carries_alpha() {
    let bytes = serve("0,0,256,256/max/30/gray.png");
    let reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    assert_eq!(reader.info().color_type, png::ColorType::GrayscaleAlpha);
}
