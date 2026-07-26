//! The in-memory raster model and the pure-compute transforms.
//!
//! M0 ships 8-bit gray and RGB; the M2 source-format matrix widens input
//! handling (16-bit, planar, subsampled YCbCr) at the decoder layer, which
//! normalizes to these working rasters.

use num_traits::cast::ToPrimitive;
use std::fmt;

/// An owned 8-bit raster, tightly packed, row-major.
///
/// The alpha variants exist for one producer — arbitrary rotation, whose
/// out-of-frame corners are transparent per the spec's recommendation —
/// and are consumed only by the encoders (PNG keeps alpha; opaque formats
/// composite over white).
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
    /// Gray + alpha, 2 bytes per pixel.
    GrayA8 {
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
    /// RGB + alpha, 4 bytes per pixel.
    Rgba8 {
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

/// BT.601 luma of one RGB pixel.
fn luma_of(r: u8, g: u8, b: u8) -> u8 {
    let luma = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
    luma.round().clamp(0.0, 255.0).to_u8().unwrap_or(255)
}

/// One channel of source-over-white compositing.
fn composite_channel(value: u8, alpha: u8) -> u8 {
    let alpha_wide = u16::from(alpha);
    let numerator = u16::from(value) * alpha_wide + 255 * (255 - alpha_wide) + 127;
    u8::try_from(numerator / 255).unwrap_or(255)
}

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
            Self::Gray8 { width, .. }
            | Self::Rgb8 { width, .. }
            | Self::GrayA8 { width, .. }
            | Self::Rgba8 { width, .. } => *width,
        }
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        match self {
            Self::Gray8 { height, .. }
            | Self::Rgb8 { height, .. }
            | Self::GrayA8 { height, .. }
            | Self::Rgba8 { height, .. } => *height,
        }
    }

    #[must_use]
    pub fn channels(&self) -> u32 {
        match self {
            Self::Gray8 { .. } => 1,
            Self::GrayA8 { .. } => 2,
            Self::Rgb8 { .. } => 3,
            Self::Rgba8 { .. } => 4,
        }
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        match self {
            Self::Gray8 { data, .. }
            | Self::Rgb8 { data, .. }
            | Self::GrayA8 { data, .. }
            | Self::Rgba8 { data, .. } => data,
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
            Self::GrayA8 { .. } => Self::GrayA8 {
                width,
                height,
                data: vec![0; pixels],
            },
            Self::Rgba8 { .. } => Self::Rgba8 {
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
        let dst_data = self.data_mut();
        for row in 0..height as usize {
            let src_off = (src_y as usize + row) * src_stride + src_x as usize * bpp;
            let dst_off = (dst_y as usize + row) * dst_stride + dst_x as usize * bpp;
            dst_data[dst_off..dst_off + row_bytes]
                .copy_from_slice(&src_data[src_off..src_off + row_bytes]);
        }
        Ok(())
    }

    fn data_mut(&mut self) -> &mut Vec<u8> {
        match self {
            Self::Gray8 { data, .. }
            | Self::Rgb8 { data, .. }
            | Self::GrayA8 { data, .. }
            | Self::Rgba8 { data, .. } => data,
        }
    }

    /// Mirror on the vertical axis (left↔right), in place.
    pub fn mirror(&mut self) {
        let width = self.width() as usize;
        let bpp = self.channels() as usize;
        let data = self.data_mut();
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
            Self::GrayA8 { .. } => Self::GrayA8 {
                width,
                height,
                data: dst,
            },
            Self::Rgba8 { .. } => Self::Rgba8 {
                width,
                height,
                data: dst,
            },
        }
    }

    fn rotate_180(&mut self) {
        let bpp = self.channels() as usize;
        let data = self.data_mut();
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
            gray @ (Self::Gray8 { .. } | Self::GrayA8 { .. }) => gray,
            Self::Rgb8 {
                width,
                height,
                data,
            } => {
                let gray = data
                    .chunks_exact(3)
                    .map(|px| luma_of(px[0], px[1], px[2]))
                    .collect();
                Self::Gray8 {
                    width,
                    height,
                    data: gray,
                }
            }
            Self::Rgba8 {
                width,
                height,
                data,
            } => {
                let gray = data
                    .chunks_exact(4)
                    .flat_map(|px| [luma_of(px[0], px[1], px[2]), px[3]])
                    .collect();
                Self::GrayA8 {
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
            Self::GrayA8 {
                width,
                height,
                mut data,
            } => {
                for px in data.chunks_exact_mut(2) {
                    px[0] = if px[0] >= 128 { 255 } else { 0 };
                }
                Self::GrayA8 {
                    width,
                    height,
                    data,
                }
            }
            other => other, // unreachable: into_gray never returns RGB
        }
    }

    /// Rotate clockwise by an arbitrary angle. The canvas grows to hold
    /// the rotated bounds; uncovered corners are transparent (the spec's
    /// recommendation) — hence the alpha output. Bilinear sampling.
    #[must_use]
    pub fn rotate_arbitrary(self, degrees: f64) -> Self {
        let theta = degrees.to_radians();
        let (sin, cos) = theta.sin_cos();
        let src_w = f64::from(self.width());
        let src_h = f64::from(self.height());
        let out_w = (src_w * cos.abs() + src_h * sin.abs()).ceil().max(1.0);
        let out_h = (src_w * sin.abs() + src_h * cos.abs()).ceil().max(1.0);
        let canvas_w = out_w.to_u32().unwrap_or(u32::MAX);
        let canvas_h = out_h.to_u32().unwrap_or(u32::MAX);
        let gray = matches!(self, Self::Gray8 { .. } | Self::GrayA8 { .. });
        let src_channels = self.channels() as usize;
        let out_channels: usize = if gray { 2 } else { 4 };
        let mut out = vec![0u8; canvas_w as usize * canvas_h as usize * out_channels];
        let source_center = (src_w / 2.0, src_h / 2.0);
        let canvas_center = (out_w / 2.0, out_h / 2.0);
        let data = self.data();
        let columns = self.width() as usize;
        let rows = self.height() as usize;
        for oy in 0..canvas_h {
            for ox in 0..canvas_w {
                // Inverse map: rotate the output pixel back by -θ around
                // the canvas center.
                let dx = f64::from(ox) + 0.5 - canvas_center.0;
                let dy = f64::from(oy) + 0.5 - canvas_center.1;
                let sx = dx * cos + dy * sin + source_center.0 - 0.5;
                let sy = -(dx * sin) + dy * cos + source_center.1 - 0.5;
                if sx < -0.5 || sy < -0.5 || sx > src_w - 0.5 || sy > src_h - 0.5 {
                    continue; // stays transparent
                }
                let x_floor = sx.floor().max(0.0);
                let y_floor = sy.floor().max(0.0);
                let fx = (sx - x_floor).clamp(0.0, 1.0);
                let fy = (sy - y_floor).clamp(0.0, 1.0);
                let x0 = x_floor.to_usize().unwrap_or(0).min(columns - 1);
                let y0 = y_floor.to_usize().unwrap_or(0).min(rows - 1);
                let x1 = (x0 + 1).min(columns - 1);
                let y1 = (y0 + 1).min(rows - 1);
                let out_off = (oy as usize * canvas_w as usize + ox as usize) * out_channels;
                for channel in 0..src_channels.min(out_channels - 1) {
                    let sample = |x: usize, y: usize| {
                        f64::from(data[(y * columns + x) * src_channels + channel])
                    };
                    let value = sample(x0, y0) * (1.0 - fx) * (1.0 - fy)
                        + sample(x1, y0) * fx * (1.0 - fy)
                        + sample(x0, y1) * (1.0 - fx) * fy
                        + sample(x1, y1) * fx * fy;
                    out[out_off + channel] = value.round().clamp(0.0, 255.0).to_u8().unwrap_or(255);
                }
                out[out_off + out_channels - 1] = 255; // opaque interior
            }
        }
        if gray {
            Self::GrayA8 {
                width: canvas_w,
                height: canvas_h,
                data: out,
            }
        } else {
            Self::Rgba8 {
                width: canvas_w,
                height: canvas_h,
                data: out,
            }
        }
    }

    /// Flatten alpha over a white background, producing an opaque raster.
    /// No-op for already-opaque rasters.
    #[must_use]
    pub fn flatten_over_white(self) -> Self {
        match self {
            opaque @ (Self::Gray8 { .. } | Self::Rgb8 { .. }) => opaque,
            Self::GrayA8 {
                width,
                height,
                data,
            } => {
                let flat = data
                    .chunks_exact(2)
                    .map(|px| composite_channel(px[0], px[1]))
                    .collect();
                Self::Gray8 {
                    width,
                    height,
                    data: flat,
                }
            }
            Self::Rgba8 {
                width,
                height,
                data,
            } => {
                let flat = data
                    .chunks_exact(4)
                    .flat_map(|px| [0, 1, 2].map(|c| composite_channel(px[c], px[3])))
                    .collect();
                Self::Rgb8 {
                    width,
                    height,
                    data: flat,
                }
            }
        }
    }
}
