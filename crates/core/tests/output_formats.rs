//! The complete output-format table: every spec format encodes, and the
//! decodable ones round-trip with correct pixels against the committed
//! deterministic fixture.

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

/// The fixture pattern: r = x % 256, g = y % 256, b = block marker.
fn expected_pixel(x: u32, y: u32) -> [u8; 3] {
    let channel = |v: u32| u8::try_from(v % 256).expect("mod 256 fits u8");
    [
        channel(x),
        channel(y),
        channel((x / 256) * 64 + (y / 256) * 32),
    ]
}

const REGION: &str = "300,200,256,256/max/0/default";

#[test]
fn tif_output_roundtrips_exactly() {
    let bytes = serve(&format!("{REGION}.tif"));
    let mut decoder = tiff::decoder::Decoder::new(Cursor::new(&bytes)).unwrap();
    let (w, h) = decoder.dimensions().unwrap();
    assert_eq!((w, h), (256, 256));
    let tiff::decoder::DecodingResult::U8(data) = decoder.read_image().unwrap() else {
        panic!("expected 8-bit output");
    };
    let off = ((100 * 256 + 100) * 3) as usize;
    assert_eq!(&data[off..off + 3], &expected_pixel(400, 300));
}

#[test]
fn webp_output_is_lossless() {
    let bytes = serve(&format!("{REGION}.webp"));
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(&bytes)).unwrap();
    assert_eq!(decoder.dimensions(), (256, 256));
    let mut data = vec![0u8; decoder.output_buffer_size().unwrap()];
    decoder.read_image(&mut data).unwrap();
    let off = ((100 * 256 + 100) * 3) as usize;
    // Lossless: exact.
    assert_eq!(&data[off..off + 3], &expected_pixel(400, 300));
}

#[test]
fn jp2_output_decodes_exactly() {
    let bytes = serve(&format!("{REGION}.jp2"));
    let mut decoder = j2k::J2kDecoder::new(&bytes).unwrap();
    assert_eq!(decoder.info().dimensions, (256, 256));
    let mut out = vec![0u8; 256 * 256 * 3];
    decoder
        .decode_into(&mut out, 256 * 3, j2k::PixelFormat::Rgb8)
        .unwrap();
    let off = ((100 * 256 + 100) * 3) as usize;
    // Reversible 5/3: exact.
    assert_eq!(&out[off..off + 3], &expected_pixel(400, 300));
}

#[test]
fn gif_output_decodes_with_palette_tolerance() {
    let bytes = serve(&format!("{REGION}.gif"));
    assert_eq!(&bytes[..6], b"GIF89a");
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(Cursor::new(&bytes)).unwrap();
    let frame = decoder.read_next_frame().unwrap().unwrap();
    assert_eq!((frame.width, frame.height), (256, 256));
    let off = ((100 * 256 + 100) * 4) as usize;
    let expected = expected_pixel(400, 300);
    for (channel, want) in expected.iter().enumerate() {
        let got = frame.buffer[off + channel];
        assert!(
            got.abs_diff(*want) <= 24,
            "palette channel {channel}: got {got}, expected ≈{want}"
        );
    }
}

#[test]
fn pdf_output_embeds_the_jpeg() {
    let bytes = serve(&format!("{REGION}.pdf"));
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(bytes.ends_with(b"%%EOF\n"));
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("/Filter /DCTDecode"));
    assert!(body.contains("/MediaBox [0 0 256 256]"));
    // The DCTDecode stream is a real JPEG (SOI marker present).
    let soi = bytes.windows(2).position(|w| w == [0xFF, 0xD8]);
    assert!(soi.is_some(), "no JPEG SOI in PDF");
}

#[test]
fn alpha_survives_webp_and_flattens_elsewhere() {
    // 45° rotation produces alpha; webp keeps it, gif/tif/pdf flatten.
    let bytes = serve("300,200,256,256/max/45/default.webp");
    let decoder = image_webp::WebPDecoder::new(Cursor::new(&bytes)).unwrap();
    assert!(decoder.has_alpha(), "webp should keep rotation alpha");
    let bytes = serve("300,200,256,256/max/45/default.tif");
    let mut decoder = tiff::decoder::Decoder::new(Cursor::new(&bytes)).unwrap();
    assert_eq!(
        decoder.colortype().unwrap(),
        tiff::ColorType::RGB(8),
        "tif flattens to opaque RGB"
    );
}
