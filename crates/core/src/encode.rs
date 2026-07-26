//! Output encoders: the complete spec table. `jpg`/`png` are the level-2
//! core; `tif`, `gif`, `jp2` (lossless), `pdf` (hand-rolled single-image
//! wrapper), and `webp` (lossless — the one documented asterisk: valid
//! `image/webp`, larger files, because lossy webp would require C libwebp)
//! complete it.

use crate::grammar::Format;
use crate::image::Raster;
use std::fmt;

/// Encoder failure. Client-caused cases (dimensions beyond a format's
/// limits) are 400s; the rest are internal.
#[derive(Debug)]
pub enum EncodeError {
    /// The output dimensions exceed what the format can represent (JPEG
    /// caps at 65535 per side).
    DimensionsBeyondFormat {
        format: Format,
        width: u32,
        height: u32,
    },
    /// Internal encoder failure.
    Internal(String),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsBeyondFormat {
                format,
                width,
                height,
            } => {
                write!(f, "{width}×{height} exceeds what {format} can represent")
            }
            Self::Internal(msg) => write!(f, "encoder failure: {msg}"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// JPEG quality used for all lossy output. Fixed: capability is baked in,
/// not toggled, and derivative caching lives at the CDN — a stable byte
/// stream per URL matters more than a knob.
const JPEG_QUALITY: u8 = 85;

/// Encode a raster in the requested format.
///
/// # Errors
///
/// See [`EncodeError`]; every format succeeds for any raster within the
/// format's own representational limits.
pub fn encode(raster: &Raster, format: Format) -> Result<Vec<u8>, EncodeError> {
    match format {
        Format::Jpg => encode_jpeg(raster),
        Format::Png => encode_png(raster),
        Format::Tif => encode_tiff(raster),
        Format::Gif => encode_gif(raster),
        Format::Webp => encode_webp(raster),
        Format::Jp2 => encode_jp2(raster),
        Format::Pdf => encode_pdf(raster),
    }
}

fn encode_tiff(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
    use tiff::encoder::{TiffEncoder, colortype};
    // Keep TIFF output opaque: alpha flattens over white.
    let raster = raster.clone().flatten_over_white();
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut encoder =
            TiffEncoder::new(&mut cursor).map_err(|e| EncodeError::Internal(e.to_string()))?;
        match &raster {
            Raster::Gray8 {
                width,
                height,
                data,
            } => encoder
                .write_image::<colortype::Gray8>(*width, *height, data)
                .map_err(|e| EncodeError::Internal(e.to_string()))?,
            Raster::Rgb8 {
                width,
                height,
                data,
            } => encoder
                .write_image::<colortype::RGB8>(*width, *height, data)
                .map_err(|e| EncodeError::Internal(e.to_string()))?,
            _ => {
                return Err(EncodeError::Internal(
                    "alpha survived flattening".to_owned(),
                ));
            }
        }
    }
    Ok(cursor.into_inner())
}

fn encode_gif(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
    // GIF is palettized and opaque here: flatten, then let the encoder
    // quantize (it switches to NeuQuant beyond 256 distinct colors).
    let raster = raster.clone().flatten_over_white();
    let width = u16::try_from(raster.width());
    let height = u16::try_from(raster.height());
    let (Ok(width), Ok(height)) = (width, height) else {
        return Err(EncodeError::DimensionsBeyondFormat {
            format: Format::Gif,
            width: raster.width(),
            height: raster.height(),
        });
    };
    let rgb;
    let pixels = match &raster {
        Raster::Rgb8 { data, .. } => data,
        Raster::Gray8 { data, .. } => {
            rgb = data
                .iter()
                .flat_map(|&px| [px, px, px])
                .collect::<Vec<u8>>();
            &rgb
        }
        _ => {
            return Err(EncodeError::Internal(
                "alpha survived flattening".to_owned(),
            ));
        }
    };
    let frame = gif::Frame::from_rgb(width, height, pixels);
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, width, height, &[])
            .map_err(|e| EncodeError::Internal(e.to_string()))?;
        encoder
            .write_frame(&frame)
            .map_err(|e| EncodeError::Internal(e.to_string()))?;
    }
    Ok(out)
}

fn encode_webp(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
    // Lossless only — the single documented asterisk in the compliance
    // table (lossy webp would require C libwebp).
    let mut out = Vec::new();
    let encoder = image_webp::WebPEncoder::new(std::io::Cursor::new(&mut out));
    let color = match raster {
        Raster::Gray8 { .. } => image_webp::ColorType::L8,
        Raster::GrayA8 { .. } => image_webp::ColorType::La8,
        Raster::Rgb8 { .. } => image_webp::ColorType::Rgb8,
        Raster::Rgba8 { .. } => image_webp::ColorType::Rgba8,
    };
    encoder
        .encode(raster.data(), raster.width(), raster.height(), color)
        .map_err(|e| EncodeError::Internal(e.to_string()))?;
    Ok(out)
}

fn encode_jp2(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
    // Reversible 5/3 lossless codestream, wrapped as a JP2 file.
    let raster = raster.clone().flatten_over_white();
    let (components, data) = match &raster {
        Raster::Gray8 { data, .. } => (1u16, data),
        Raster::Rgb8 { data, .. } => (3u16, data),
        _ => {
            return Err(EncodeError::Internal(
                "alpha survived flattening".to_owned(),
            ));
        }
    };
    let samples = j2k::J2kLosslessSamples {
        data,
        width: raster.width(),
        height: raster.height(),
        components,
        bit_depth: 8,
        signed: false,
    };
    let options = j2k::J2kLosslessEncodeOptions::default();
    let encoded = j2k::encode_j2k_lossless(samples, &options)
        .map_err(|e| EncodeError::Internal(e.to_string()))?;
    let wrapped = j2k::wrap_j2k_codestream(&encoded.codestream, j2k::J2kFileWrapOptions::jp2())
        .map_err(|e| EncodeError::Internal(e.to_string()))?;
    Ok(wrapped)
}

/// Hand-rolled single-image PDF (design decision: ~150 lines beat a
/// dependency). The page embeds the pipeline's JPEG output via `DCTDecode`
/// at 72 dpi, sized 1 pt per pixel.
fn encode_pdf(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
    let raster = raster.clone().flatten_over_white();
    let jpeg = encode_jpeg(&raster)?;
    let (width, height) = (raster.width(), raster.height());
    let colorspace = match &raster {
        Raster::Gray8 { .. } => "/DeviceGray",
        _ => "/DeviceRGB",
    };
    let content = format!(
        "q
{width} 0 0 {height} 0 0 cm
/Im0 Do
Q
"
    );

    let mut pdf: Vec<u8> = Vec::with_capacity(jpeg.len() + 1024);
    let mut offsets = [0usize; 6];
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let objects: [(usize, Vec<u8>); 5] = [
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}]                 /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>"
            )
            .into_bytes(),
        ),
        (4, {
            let mut object = format!(
                "<< /Type /XObject /Subtype /Image /Width {width} /Height {height}                 /ColorSpace {colorspace} /BitsPerComponent 8 /Filter /DCTDecode                 /Length {} >>
stream
",
                jpeg.len()
            )
            .into_bytes();
            object.extend_from_slice(&jpeg);
            object.extend_from_slice(b"
endstream");
            object
        }),
        (
            5,
            format!("<< /Length {} >>
stream
{content}endstream", content.len()).into_bytes(),
        ),
    ];
    for (number, body) in &objects {
        offsets[*number] = pdf.len();
        pdf.extend_from_slice(
            format!(
                "{number} 0 obj
"
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(
            b"
endobj
",
        );
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(
        b"xref
0 6
0000000000 65535 f
",
    );
    for offset in &offsets[1..] {
        pdf.extend_from_slice(
            format!(
                "{offset:010} 00000 n
"
            )
            .as_bytes(),
        );
    }
    pdf.extend_from_slice(
        format!(
            "trailer
<< /Size 6 /Root 1 0 R >>
startxref
{xref_offset}
%%EOF
"
        )
        .as_bytes(),
    );
    Ok(pdf)
}

fn encode_jpeg(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
    // JPEG is opaque: flatten any alpha over white first.
    let flattened;
    let raster = match raster {
        Raster::GrayA8 { .. } | Raster::Rgba8 { .. } => {
            flattened = raster.clone().flatten_over_white();
            &flattened
        }
        opaque => opaque,
    };
    let width = u16::try_from(raster.width());
    let height = u16::try_from(raster.height());
    let (Ok(width), Ok(height)) = (width, height) else {
        return Err(EncodeError::DimensionsBeyondFormat {
            format: Format::Jpg,
            width: raster.width(),
            height: raster.height(),
        });
    };
    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, JPEG_QUALITY);
    let color = match raster {
        Raster::Gray8 { .. } => jpeg_encoder::ColorType::Luma,
        Raster::Rgb8 { .. } => jpeg_encoder::ColorType::Rgb,
        // Unreachable after flattening; keep the match total.
        Raster::GrayA8 { .. } | Raster::Rgba8 { .. } => {
            return Err(EncodeError::Internal(
                "alpha survived flattening".to_owned(),
            ));
        }
    };
    encoder
        .encode(raster.data(), width, height, color)
        .map_err(|e| EncodeError::Internal(e.to_string()))?;
    Ok(out)
}

fn encode_png(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, raster.width(), raster.height());
        encoder.set_color(match raster {
            Raster::Gray8 { .. } => png::ColorType::Grayscale,
            Raster::Rgb8 { .. } => png::ColorType::Rgb,
            Raster::GrayA8 { .. } => png::ColorType::GrayscaleAlpha,
            Raster::Rgba8 { .. } => png::ColorType::Rgba,
        });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| EncodeError::Internal(e.to_string()))?;
        writer
            .write_image_data(raster.data())
            .map_err(|e| EncodeError::Internal(e.to_string()))?;
    }
    Ok(out)
}
