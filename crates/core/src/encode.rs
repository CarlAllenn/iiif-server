//! Output encoders. Level 2 requires `jpg` and `png`; the remaining table
//! (`tif`, `jp2`, `gif`, `pdf`, `webp`-lossless) lands in the
//! completionist sweep behind the same function.

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
    /// The format is spec-legal but this binary does not encode it (yet):
    /// HTTP 400 per §4.5.
    UnsupportedFormat(Format),
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
            Self::UnsupportedFormat(format) => {
                write!(f, "format {format} is not supported by this build")
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
/// See [`EncodeError`]; `jpg`/`png` succeed for any raster within format
/// limits, other formats return `UnsupportedFormat` until the
/// completionist sweep ships them.
pub fn encode(raster: &Raster, format: Format) -> Result<Vec<u8>, EncodeError> {
    match format {
        Format::Jpg => encode_jpeg(raster),
        Format::Png => encode_png(raster),
        other => Err(EncodeError::UnsupportedFormat(other)),
    }
}

fn encode_jpeg(raster: &Raster) -> Result<Vec<u8>, EncodeError> {
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
