//! End-to-end pixel path against the committed deterministic fixture:
//! open a pyramidal TIFF, derive info.json from its real structure,
//! decode a region, resize, transform, encode — and verify actual pixel
//! values (the fixture pattern encodes each pixel's coordinates).

use iiif_core::codec::TiffPyramid;
use iiif_core::eval::evaluate;
use iiif_core::grammar::ImageRequest;
use iiif_core::image::Raster;
use iiif_core::info::{Info, Limits};
use iiif_core::pipeline;
use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;

const LIMITS: Limits = Limits {
    max_width: 8192,
    max_height: 8192,
    max_area: 67_108_864,
};

fn fixture() -> TiffPyramid<File> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/rgb_pyramid.tif");
    TiffPyramid::open(File::open(path).expect("fixture present")).expect("valid pyramid")
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

#[test]
fn pyramid_structure_is_surveyed() {
    let tiff = fixture();
    assert_eq!(tiff.dimensions(), (1024, 768));
    let levels = tiff.levels();
    assert_eq!(levels.len(), 3, "vips halves 1024→512→256");
    assert_eq!(levels[0].scale_factor, 1);
    assert_eq!(levels[1].scale_factor, 2);
    assert_eq!(levels[2].scale_factor, 4);
    assert_eq!((levels[0].tile_width, levels[0].tile_height), (256, 256));
}

#[test]
fn info_json_derives_from_actual_pyramid() {
    let tiff = fixture();
    let description = tiff.describe();
    let info = Info::new(
        "https://example.org/iiif/3/rgb_pyramid.tif".to_owned(),
        &description,
        LIMITS,
    );
    let json: serde_json::Value = serde_json::from_str(&info.to_json()).unwrap();
    assert_eq!(json["@context"], "http://iiif.io/api/image/3/context.json");
    assert_eq!(json["type"], "ImageService3");
    assert_eq!(json["protocol"], "http://iiif.io/api/image");
    assert_eq!(json["profile"], "level2");
    assert_eq!(json["width"], 1024);
    assert_eq!(json["height"], 768);
    assert_eq!(json["maxWidth"], 8192);
    assert_eq!(json["maxArea"], 67_108_864);
    // tiles derived from the real pyramid: 256px tiles at 1/2/4.
    assert_eq!(json["tiles"][0]["width"], 256);
    assert_eq!(
        json["tiles"][0]["scaleFactors"],
        serde_json::json!([1, 2, 4])
    );
    // sizes ascending, one per level.
    assert_eq!(
        json["sizes"],
        serde_json::json!([
            {"width": 256, "height": 192},
            {"width": 512, "height": 384},
            {"width": 1024, "height": 768},
        ])
    );
}

#[test]
fn full_region_decode_matches_pattern() {
    let mut tiff = fixture();
    let raster = tiff.decode_region(0, 0, 0, 1024, 768).unwrap();
    assert_eq!((raster.width(), raster.height()), (1024, 768));
    let Raster::Rgb8 { data, .. } = &raster else {
        panic!("fixture is RGB")
    };
    // Spot-check pixels across tile boundaries.
    for (x, y) in [
        (0, 0),
        (255, 255),
        (256, 256),
        (511, 300),
        (1023, 767),
        (700, 10),
    ] {
        let off = ((y * 1024 + x) * 3) as usize;
        assert_eq!(
            [data[off], data[off + 1], data[off + 2]],
            expected_pixel(x, y),
            "pixel at ({x},{y})"
        );
    }
}

#[test]
fn sub_tile_region_decode_is_exact() {
    let mut tiff = fixture();
    // A region straddling four tiles: (200..520, 200..460).
    let raster = tiff.decode_region(0, 200, 200, 320, 260).unwrap();
    assert_eq!((raster.width(), raster.height()), (320, 260));
    let Raster::Rgb8 { data, .. } = &raster else {
        panic!("fixture is RGB")
    };
    for (rx, ry) in [(0, 0), (319, 259), (56, 56), (57, 57), (200, 100)] {
        let off = ((ry * 320 + rx) * 3) as usize;
        assert_eq!(
            [data[off], data[off + 1], data[off + 2]],
            expected_pixel(200 + rx, 200 + ry),
            "region pixel at ({rx},{ry})"
        );
    }
}

#[test]
fn one_real_tile_decoded_resized_encoded() {
    // The M0 acceptance demo: a native 256px tile, downscaled, as JPEG
    // and PNG.
    let mut tiff = fixture();
    let request = ImageRequest::parse("256,256,256,256/128,/0/default.png").unwrap();
    let plan = evaluate(&request, 1024, 768, LIMITS).unwrap();
    assert_eq!((plan.out_w, plan.out_h), (128, 128));

    let bytes = pipeline::execute(&mut tiff, &plan).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (128, 128));
    assert_eq!(info.color_type, png::ColorType::Rgb);

    // The pattern is linear in x (r channel) and y (g channel), so
    // Lanczos downscale-by-2 must land close to the source midpoint
    // value. Check the raster's own coordinate mapping: output (ox, oy)
    // ↔ source (256 + 2·ox, 256 + 2·oy).
    for (ox, oy) in [(32, 32), (64, 64), (100, 20)] {
        let off = ((oy * 128 + ox) * 3) as usize;
        let expected = expected_pixel(256 + 2 * ox, 256 + 2 * oy);
        let got = [buf[off], buf[off + 1], buf[off + 2]];
        for channel in 0..2 {
            let delta = i16::from(got[channel]) - i16::from(expected[channel]);
            assert!(
                delta.abs() <= 3,
                "channel {channel} at ({ox},{oy}): got {got:?}, expected ≈{expected:?}"
            );
        }
    }

    // Same request as JPEG: decodes and has the right dimensions.
    let request = ImageRequest::parse("256,256,256,256/128,/0/default.jpg").unwrap();
    let plan = evaluate(&request, 1024, 768, LIMITS).unwrap();
    let bytes = pipeline::execute(&mut tiff, &plan).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "JPEG SOI marker");
    let mut zune = zune_jpeg::JpegDecoder::new(Cursor::new(&bytes));
    let pixels = zune.decode().unwrap();
    let dims = zune.dimensions().unwrap();
    assert_eq!(dims, (128, 128));
    assert!(!pixels.is_empty());
}

#[test]
fn quality_and_rotation_transforms() {
    let mut tiff = fixture();
    // Gray quality produces single-channel PNG.
    let request = ImageRequest::parse("0,0,256,256/64,/0/gray.png").unwrap();
    let plan = evaluate(&request, 1024, 768, LIMITS).unwrap();
    let bytes = pipeline::execute(&mut tiff, &plan).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Grayscale);

    // Bitonal: only 0 and 255 survive.
    let request = ImageRequest::parse("0,0,256,256/64,/0/bitonal.png").unwrap();
    let plan = evaluate(&request, 1024, 768, LIMITS).unwrap();
    let bytes = pipeline::execute(&mut tiff, &plan).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert!(
        buf[..info.buffer_size()]
            .iter()
            .all(|&px| px == 0 || px == 255)
    );

    // 90° rotation swaps dimensions.
    let request = ImageRequest::parse("0,0,512,256/256,128/90/default.png").unwrap();
    let plan = evaluate(&request, 1024, 768, LIMITS).unwrap();
    let bytes = pipeline::execute(&mut tiff, &plan).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (128, 256));
}

#[test]
fn level_selection_uses_pyramid() {
    let tiff = fixture();
    // Full image at 256 wide → downscale factor 4 → deepest level.
    assert_eq!(tiff.level_for_scale(4.0).scale_factor, 4);
    // Factor 3 → level 2 (enough detail), not level 4.
    assert_eq!(tiff.level_for_scale(3.0).scale_factor, 2);
    assert_eq!(tiff.level_for_scale(1.0).scale_factor, 1);
    // Upscales still read level 1.
    assert_eq!(tiff.level_for_scale(0.5).scale_factor, 1);
}

#[test]
fn mirroring_reverses_rows() {
    let mut tiff = fixture();
    let request = ImageRequest::parse("0,0,256,256/256,/!0/default.png").unwrap();
    let plan = evaluate(&request, 1024, 768, LIMITS).unwrap();
    let bytes = pipeline::execute(&mut tiff, &plan).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut buf).unwrap();
    // Mirrored: output x → source x' = 255 - x, so r channel runs 255→0.
    assert_eq!(buf[0], 255, "left edge r channel after mirror");
    let last = (255 * 3) as usize;
    assert_eq!(buf[last], 0, "right edge r channel after mirror");
}
