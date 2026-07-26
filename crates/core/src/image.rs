//! The in-memory raster model and the pure-compute transforms.
//!
//! M0 ships 8-bit gray and RGB; the M2 source-format matrix widens input
//! handling (16-bit, planar, subsampled YCbCr) at the decoder layer, which
//! normalizes to these working rasters.

use num_traits::cast::ToPrimitive;
use std::fmt;

/// An owned 8-bit raster, tightly packed, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raster {
    /// Single channel, 1 byte per pixel.
    Gray8 {
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
    /// Three channels, RGB order, 3 bytes per pixel.
    Rgb8 {
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
}

/// Pixel-geometry or buffer-consistency failure inside the pipeline —
/// always an internal bug or a decoder contract violation, never a client
/// error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterError(pub String);

impl fmt::Display for RasterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "raster error: {}", self.0)
    }
}

impl std::error::Error for RasterError {}

/// A source rectangle for [`Raster::blit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyRect {
    pub src_x: u32,
    pub src_y: u32,
    pub width: u32,
    pub height: u32,
}

impl Raster {
    #[must_use]
    pub fn width(&self) -> u32 {
        match self {
            Self::Gray8 { width, .. } | Self::Rgb8 { width, .. } => *width,
        }
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        match self {
            Self::Gray8 { height, .. } | Self::Rgb8 { height, .. } => *height,
        }
    }

    #[must_use]
    pub fn channels(&self) -> u32 {
        match self {
            Self::Gray8 { .. } => 1,
            Self::Rgb8 { .. } => 3,
        }
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        match self {
            Self::Gray8 { data, .. } | Self::Rgb8 { data, .. } => data,
        }
    }

    /// Allocate a zeroed raster with the same pixel layout.
    ///
    /// # Errors
    ///
    /// Fails when `width * height * channels` overflows `usize` — the
    /// per-decode allocation ceilings upstream make this unreachable in
    /// practice, but the arithmetic stays checked.
    pub fn zeroed_like(&self, width: u32, height: u32) -> Result<Self, RasterError> {
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(self.channels() as usize))
            .ok_or_else(|| RasterError("allocation size overflow".to_owned()))?;
        Ok(match self {
            Self::Gray8 { .. } => Self::Gray8 {
                width,
                height,
                data: vec![0; pixels],
            },
            Self::Rgb8 { .. } => Self::Rgb8 {
                width,
                height,
                data: vec![0; pixels],
            },
        })
    }

    /// Copy `rect` of `src` into this raster at (`dst_x`, `dst_y`).
    /// Layouts must match.
    ///
    /// # Errors
    ///
    /// Fails when the rectangles fall outside either raster or the pixel
    /// layouts differ.
    pub fn blit(
        &mut self,
        src: &Self,
        rect: CopyRect,
        dst_x: u32,
        dst_y: u32,
    ) -> Result<(), RasterError> {
        if self.channels() != src.channels() {
            return Err(RasterError(
                "blit between different pixel layouts".to_owned(),
            ));
        }
        let CopyRect {
            src_x,
            src_y,
            width,
            height,
        } = rect;
        if src_x
            .checked_add(width)
            .is_none_or(|edge| edge > src.width())
            || src_y
                .checked_add(height)
                .is_none_or(|edge| edge > src.height())
            || dst_x
                .checked_add(width)
                .is_none_or(|edge| edge > self.width())
            || dst_y
                .checked_add(height)
                .is_none_or(|edge| edge > self.height())
        {
            return Err(RasterError("blit rectangle out of bounds".to_owned()));
        }
        let bpp = self.channels() as usize;
        let src_stride = src.width() as usize * bpp;
        let dst_stride = self.width() as usize * bpp;
        let row_bytes = width as usize * bpp;
        let src_data = src.data();
        let dst_data = match self {
            Self::Gray8 { data, .. } | Self::Rgb8 { data, .. } => data,
        };
        for row in 0..height as usize {
            let src_off = (src_y as usize + row) * src_stride + src_x as usize * bpp;
            let dst_off = (dst_y as usize + row) * dst_stride + dst_x as usize * bpp;
            dst_data[dst_off..dst_off + row_bytes]
                .copy_from_slice(&src_data[src_off..src_off + row_bytes]);
        }
        Ok(())
    }

    /// Mirror on the vertical axis (left↔right), in place.
    pub fn mirror(&mut self) {
        let width = self.width() as usize;
        let bpp = self.channels() as usize;
        let data = match self {
            Self::Gray8 { data, .. } | Self::Rgb8 { data, .. } => data,
        };
        for row in data.chunks_exact_mut(width * bpp) {
            let mut left = 0;
            let mut right = width - 1;
            while left < right {
                for byte in 0..bpp {
                    row.swap(left * bpp + byte, right * bpp + byte);
                }
                left += 1;
                right -= 1;
            }
        }
    }

    /// Rotate clockwise by the given number of quarter turns (0–3).
    #[must_use]
    pub fn rotate_quarters(self, quarters: u8) -> Self {
        match quarters % 4 {
            1 => self.rotated_90(),
            2 => {
                let mut out = self;
                out.rotate_180();
                out
            }
            3 => {
                let mut out = self.rotated_90();
                out.rotate_180();
                out
            }
            _ => self,
        }
    }

    fn rotated_90(self) -> Self {
        let src_w = self.width() as usize;
        let src_h = self.height() as usize;
        let bpp = self.channels() as usize;
        let src = self.data();
        let mut dst = vec![0u8; src.len()];
        // (x, y) → (dst_x, dst_y) = (src_h - 1 - y, x); dst is src_h wide.
        for y in 0..src_h {
            for x in 0..src_w {
                let from = (y * src_w + x) * bpp;
                let to = (x * src_h + (src_h - 1 - y)) * bpp;
                dst[to..to + bpp].copy_from_slice(&src[from..from + bpp]);
            }
        }
        let (width, height) = (self.height(), self.width());
        match self {
            Self::Gray8 { .. } => Self::Gray8 {
                width,
                height,
                data: dst,
            },
            Self::Rgb8 { .. } => Self::Rgb8 {
                width,
                height,
                data: dst,
            },
        }
    }

    fn rotate_180(&mut self) {
        let bpp = self.channels() as usize;
        let data = match self {
            Self::Gray8 { data, .. } | Self::Rgb8 { data, .. } => data,
        };
        let pixels = data.len() / bpp;
        for i in 0..pixels / 2 {
            let j = pixels - 1 - i;
            for byte in 0..bpp {
                data.swap(i * bpp + byte, j * bpp + byte);
            }
        }
    }

    /// Convert to grayscale (BT.601 luma), a no-op for gray input.
    #[must_use]
    pub fn into_gray(self) -> Self {
        match self {
            gray @ Self::Gray8 { .. } => gray,
            Self::Rgb8 {
                width,
                height,
                data,
            } => {
                let gray = data
                    .chunks_exact(3)
                    .map(|px| {
                        let luma = 0.299 * f64::from(px[0])
                            + 0.587 * f64::from(px[1])
                            + 0.114 * f64::from(px[2]);
                        luma.round().clamp(0.0, 255.0).to_u8().unwrap_or(255)
                    })
                    .collect();
                Self::Gray8 {
                    width,
                    height,
                    data: gray,
                }
            }
        }
    }

    /// Convert to bitonal: grayscale, then a 50% threshold to pure
    /// black/white.
    #[must_use]
    pub fn into_bitonal(self) -> Self {
        match self.into_gray() {
            Self::Gray8 {
                width,
                height,
                mut data,
            } => {
                for px in &mut data {
                    *px = if *px >= 128 { 255 } else { 0 };
                }
                Self::Gray8 {
                    width,
                    height,
                    data,
                }
            }
            rgb @ Self::Rgb8 { .. } => rgb, // unreachable: into_gray never returns Rgb8
        }
    }
}
