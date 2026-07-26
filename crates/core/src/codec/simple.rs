//! Plain JPEG and PNG masters: decoded whole (they have no pyramid or
//! tiles to exploit), served by cropping the resident raster. `check`
//! advises converting large ones to pyramids; small images are fine here.

use super::{CodecError, Master};
use crate::eval::CropRect;
use crate::image::{CopyRect, Raster};
use crate::info::ImageDescription;
use std::io::Cursor;

/// A fully decoded single-resolution master.
pub struct SimpleMaster {
    raster: Raster,
}

impl SimpleMaster {
    /// Decode a plain JPEG master (incl. CMYK/YCCK via zune-jpeg's
    /// conversion to RGB).
    ///
    /// # Errors
    ///
    /// [`CodecError::Corrupt`] when the stream does not decode;
    /// [`CodecError::Unsupported`] for sample layouts outside the matrix.
    pub fn from_jpeg(bytes: &[u8]) -> Result<Self, CodecError> {
        use zune_jpeg::zune_core::colorspace::ColorSpace;
        use zune_jpeg::zune_core::options::DecoderOptions;
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
            zune_jpeg::zune_core::bytestream::ZCursor::new(bytes),
            options,
        );
        let pixels = decoder
            .decode()
            .map_err(|e| CodecError::Corrupt(format!("JPEG decode: {e}")))?;
        let (width, height) = decoder
            .dimensions()
            .ok_or_else(|| CodecError::Corrupt("JPEG has no dimensions".to_owned()))?;
        let (width, height) = (
            u32::try_from(width).map_err(|_| CodecError::Corrupt("width overflow".to_owned()))?,
            u32::try_from(height).map_err(|_| CodecError::Corrupt("height overflow".to_owned()))?,
        );
        Ok(Self {
            raster: Raster::Rgb8 {
                width,
                height,
                data: pixels,
            },
        })
    }

    /// Decode a PNG master (gray, RGB; palette and 16-bit arrive with the
    /// M2 matrix work; alpha is composited over white).
    ///
    /// # Errors
    ///
    /// [`CodecError::Corrupt`] / [`CodecError::Unsupported`] as above.
    pub fn from_png(bytes: &[u8]) -> Result<Self, CodecError> {
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder
            .read_info()
            .map_err(|e| CodecError::Corrupt(format!("PNG decode: {e}")))?;
        let mut buf = vec![
            0u8;
            reader
                .output_buffer_size()
                .ok_or_else(|| CodecError::Corrupt("PNG size overflow".to_owned()))?
        ];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| CodecError::Corrupt(format!("PNG decode: {e}")))?;
        buf.truncate(info.buffer_size());
        let (width, height) = (info.width, info.height);
        let raster = match (info.color_type, info.bit_depth) {
            (png::ColorType::Grayscale, png::BitDepth::Eight) => Raster::Gray8 {
                width,
                height,
                data: buf,
            },
            (png::ColorType::Rgb, png::BitDepth::Eight) => Raster::Rgb8 {
                width,
                height,
                data: buf,
            },
            (png::ColorType::Rgba, png::BitDepth::Eight) => {
                // Composite over white: IIIF output formats are opaque
                // (jpg) or would just carry the flattening anyway.
                let data = buf
                    .chunks_exact(4)
                    .flat_map(|px| {
                        let alpha = u16::from(px[3]);
                        [0, 1, 2].map(|channel| {
                            let value = u16::from(px[channel]);
                            u8::try_from((value * alpha + 255 * (255 - alpha) + 127) / 255)
                                .unwrap_or(255)
                        })
                    })
                    .collect();
                Raster::Rgb8 {
                    width,
                    height,
                    data,
                }
            }
            (color, depth) => {
                return Err(CodecError::Unsupported(format!(
                    "PNG {color:?}/{depth:?} is not yet in the supported matrix"
                )));
            }
        };
        Ok(Self { raster })
    }

    /// Wrap an already-decoded raster (used by tests).
    #[must_use]
    pub fn from_raster(raster: Raster) -> Self {
        Self { raster }
    }
}

impl Master for SimpleMaster {
    fn dimensions(&self) -> (u32, u32) {
        (self.raster.width(), self.raster.height())
    }

    fn describe(&self) -> ImageDescription {
        // No pyramid, no tiles: sizes lists the one complete size. Honest
        // structure — viewers fall back to whole-image requests.
        ImageDescription {
            width: self.raster.width(),
            height: self.raster.height(),
            tiles: Vec::new(),
            sizes: vec![crate::info::SizeEntry {
                width: self.raster.width(),
                height: self.raster.height(),
            }],
        }
    }

    fn decode_crop(&mut self, crop: CropRect, _needed: f64) -> Result<Raster, CodecError> {
        let mut out = self.raster.zeroed_like(crop.w, crop.h)?;
        out.blit(
            &self.raster,
            CopyRect {
                src_x: crop.x,
                src_y: crop.y,
                width: crop.w,
                height: crop.h,
            },
            0,
            0,
        )?;
        Ok(out)
    }
}
