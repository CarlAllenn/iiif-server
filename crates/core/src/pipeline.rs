//! Plan execution: level selection → region decode → resample → quality →
//! mirror/rotate → encode. Pure compute; the caller owns threading and
//! backpressure.

use crate::codec::{CodecError, Master};
use crate::encode::{EncodeError, encode};
use crate::eval::Plan;
use crate::grammar::Quality;
use crate::image::{Raster, RasterError};
use fast_image_resize as fir;
use num_traits::cast::ToPrimitive;
use std::fmt;

/// Pipeline failure, split by who caused it.
#[derive(Debug)]
pub enum PipelineError {
    Codec(CodecError),
    Encode(EncodeError),
    Raster(RasterError),
    Resize(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "{e}"),
            Self::Encode(e) => write!(f, "{e}"),
            Self::Raster(e) => write!(f, "{e}"),
            Self::Resize(msg) => write!(f, "resample failure: {msg}"),
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<CodecError> for PipelineError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e)
    }
}

impl From<EncodeError> for PipelineError {
    fn from(e: EncodeError) -> Self {
        Self::Encode(e)
    }
}

impl From<RasterError> for PipelineError {
    fn from(e: RasterError) -> Self {
        Self::Raster(e)
    }
}

/// Execute a plan against an opened master, returning encoded bytes.
///
/// # Errors
///
/// See [`PipelineError`].
pub fn execute(source: &mut dyn Master, plan: &Plan) -> Result<Vec<u8>, PipelineError> {
    // 1. Decode the crop with enough detail for the output scale; the
    //    codec picks its own cheapest path (pyramid level, reduced-
    //    resolution wavelet decode, or resident raster).
    let needed = f64::from(plan.crop.w) / f64::from(plan.out_w.max(1));
    let raster = source.decode_crop(plan.crop, needed)?;

    // 2. Resample to the output size.
    let raster = resize(raster, plan.out_w, plan.out_h)?;

    // 3. Quality.
    let raster = match plan.quality {
        Quality::Default | Quality::Color => raster,
        Quality::Gray => raster.into_gray(),
        Quality::Bitonal => raster.into_bitonal(),
    };

    // 4. Mirror, then rotate.
    let mut raster = raster;
    if plan.mirror {
        raster.mirror();
    }
    let raster = if plan.degrees == 0.0 {
        raster
    } else if plan.degrees % 90.0 == 0.0 {
        raster.rotate_quarters((plan.degrees / 90.0).to_u8().unwrap_or(0))
    } else {
        raster.rotate_arbitrary(plan.degrees)
    };

    // 5. Encode.
    Ok(encode(&raster, plan.format)?)
}

/// Lanczos3 resample via `fast_image_resize`; identity sizes short-circuit.
fn resize(raster: Raster, out_w: u32, out_h: u32) -> Result<Raster, PipelineError> {
    if raster.width() == out_w && raster.height() == out_h {
        return Ok(raster);
    }
    let pixel_type = match &raster {
        Raster::Gray8 { .. } => fir::PixelType::U8,
        Raster::GrayA8 { .. } => fir::PixelType::U8x2,
        Raster::Rgb8 { .. } => fir::PixelType::U8x3,
        Raster::Rgba8 { .. } => fir::PixelType::U8x4,
    };
    let channels = raster.channels();
    let (width, height, data) = match raster {
        Raster::Gray8 {
            width,
            height,
            data,
        }
        | Raster::Rgb8 {
            width,
            height,
            data,
        }
        | Raster::GrayA8 {
            width,
            height,
            data,
        }
        | Raster::Rgba8 {
            width,
            height,
            data,
        } => (width, height, data),
    };
    let src = fir::images::Image::from_vec_u8(width, height, data, pixel_type)
        .map_err(|e| PipelineError::Resize(e.to_string()))?;
    let mut dst = fir::images::Image::new(out_w, out_h, pixel_type);
    let mut resizer = fir::Resizer::new();
    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Lanczos3));
    resizer
        .resize(&src, &mut dst, &options)
        .map_err(|e| PipelineError::Resize(e.to_string()))?;
    let data = dst.into_vec();
    Ok(match channels {
        1 => Raster::Gray8 {
            width: out_w,
            height: out_h,
            data,
        },
        2 => Raster::GrayA8 {
            width: out_w,
            height: out_h,
            data,
        },
        3 => Raster::Rgb8 {
            width: out_w,
            height: out_h,
            data,
        },
        _ => Raster::Rgba8 {
            width: out_w,
            height: out_h,
            data,
        },
    })
}
