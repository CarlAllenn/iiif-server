//! JP2 / HTJ2K masters via the pure-Rust `j2k` crate — the capability no
//! `OpenJPEG`-based incumbent has, validated bit-exact against `OpenJPEG` by
//! SPIKE 2.

use super::{CodecError, Master};
use crate::eval::CropRect;
use crate::image::Raster;
use crate::info::{ImageDescription, SizeEntry, TileSet};
use j2k::{CpuDecodeParallelism, Downscale, J2kDecoder, J2kScratchPool, PixelFormat, Rect};

/// Wrap raw interleaved samples in the right raster variant.
fn raster_of(fmt: PixelFormat, width: u32, height: u32, data: Vec<u8>) -> Raster {
    match fmt {
        PixelFormat::Gray8 => Raster::Gray8 {
            width,
            height,
            data,
        },
        _ => Raster::Rgb8 {
            width,
            height,
            data,
        },
    }
}

/// Default tile size advertised for untiled codestreams: reduced-
/// resolution decode makes any aligned request natively cheap, so the
/// advertised grid is a viewer hint, not a constraint.
const DEFAULT_TILE: u32 = 1024;

/// An opened JP2/HTJ2K master. Owns the compressed bytes; decoders borrow
/// them per request (parse state is cheap relative to pixel work, and a
/// fresh decoder per decode keeps the type `Send` for the worker pool).
pub struct Jp2Master {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    components: u16,
    resolution_levels: u8,
    tile: (u32, u32),
    /// Whether the image dimensions are exact multiples of the tile size.
    /// j2k 0.7.5's region decode returns wrong pixels on grids with
    /// partial edge tiles (verified against `OpenJPEG` goldens, 2026-07-26;
    /// whole-image decode is correct at every scale) — such masters take
    /// the decode-full-then-crop path until upstream fixes region reads.
    exact_grid: bool,
}

impl Jp2Master {
    /// Parse the header and survey the codestream structure.
    ///
    /// # Errors
    ///
    /// [`CodecError::Corrupt`] when the stream does not parse;
    /// [`CodecError::Unsupported`] for component layouts outside the
    /// current matrix.
    pub fn new(bytes: Vec<u8>) -> Result<Self, CodecError> {
        let decoder =
            J2kDecoder::new(&bytes).map_err(|e| CodecError::Corrupt(format!("JP2 parse: {e}")))?;
        let info = decoder.info();
        let (width, height) = info.dimensions;
        let components = info.components;
        if !matches!(components, 1 | 3) {
            return Err(CodecError::Unsupported(format!(
                "{components}-component JP2 is not yet in the supported matrix"
            )));
        }
        let resolution_levels = info.resolution_levels.max(1);
        let tile = info
            .tile_layout
            .as_ref()
            .map_or((DEFAULT_TILE, DEFAULT_TILE), |t| {
                (t.tile_width, t.tile_height)
            });
        let exact_grid =
            info.tile_layout.is_none() || (width % tile.0 == 0 && height % tile.1 == 0);
        Ok(Self {
            bytes,
            width,
            height,
            components,
            resolution_levels,
            tile,
            exact_grid,
        })
    }

    fn pixel_format(&self) -> PixelFormat {
        if self.components == 1 {
            PixelFormat::Gray8
        } else {
            PixelFormat::Rgb8
        }
    }

    /// The deepest cheap downscale: bounded by the codestream's own
    /// resolution ladder and by the decode API's 1/8 ceiling (SPIKE 2
    /// finding — deeper zoom-outs decode at 1/8 and resample).
    fn downscale_for(&self, needed: f64) -> Downscale {
        let max_level = u32::from(self.resolution_levels - 1).min(3);
        let mut choice = Downscale::None;
        for (level, candidate) in [
            (1u32, Downscale::Half),
            (2, Downscale::Quarter),
            (3, Downscale::Eighth),
        ] {
            if level <= max_level && f64::from(candidate.denominator()) <= needed {
                choice = candidate;
            }
        }
        choice
    }
}

impl Master for Jp2Master {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn describe(&self) -> ImageDescription {
        // scaleFactors mirror the codestream's real resolution ladder.
        let scale_factors: Vec<u32> = (0..self.resolution_levels)
            .map(|level| 1u32 << level)
            .collect();
        let mut sizes: Vec<SizeEntry> = scale_factors
            .iter()
            .map(|factor| SizeEntry {
                width: self.width.div_ceil(*factor),
                height: self.height.div_ceil(*factor),
            })
            .collect();
        sizes.reverse();
        ImageDescription {
            width: self.width,
            height: self.height,
            tiles: vec![TileSet {
                width: self.tile.0,
                height: if self.tile.1 == self.tile.0 {
                    None
                } else {
                    Some(self.tile.1)
                },
                scale_factors,
            }],
            sizes,
        }
    }

    fn decode_crop(&mut self, crop: CropRect, needed: f64) -> Result<Raster, CodecError> {
        let mut decoder = J2kDecoder::new(&self.bytes)
            .map_err(|e| CodecError::Corrupt(format!("JP2 parse: {e}")))?;
        // The engine's worker pool owns concurrency (SPIKE 2: Serial costs
        // ~nothing on the region path and prevents pool oversubscription).
        decoder.set_cpu_decode_parallelism(CpuDecodeParallelism::Serial);
        let scale = self.downscale_for(needed);
        let fmt = self.pixel_format();
        let bpp = match fmt {
            PixelFormat::Gray8 => 1usize,
            _ => 3usize,
        };
        let roi = Rect {
            x: crop.x,
            y: crop.y,
            w: crop.w,
            h: crop.h,
        };
        let scaled = roi.scaled_covering(scale);
        let mut pool = J2kScratchPool::new();
        if self.exact_grid {
            let stride = scaled.w as usize * bpp;
            let mut out = vec![0u8; stride * scaled.h as usize];
            decoder
                .decode_region_scaled_into(&mut pool, &mut out, stride, fmt, roi, scale)
                .map_err(|e| CodecError::Corrupt(format!("JP2 decode: {e}")))?;
            return Ok(raster_of(fmt, scaled.w, scaled.h, out));
        }
        // Partial-grid fallback: whole image at the chosen scale, then
        // crop in raster space. Correct (verified against `OpenJPEG`);
        // costs a full decode — the `check` subcommand will flag such
        // masters until the upstream region-decode fix lands.
        let full_w = self.width.div_ceil(scale.denominator());
        let full_h = self.height.div_ceil(scale.denominator());
        let stride = full_w as usize * bpp;
        let mut out = vec![0u8; stride * full_h as usize];
        decoder
            .decode_scaled_into(&mut pool, &mut out, stride, fmt, scale)
            .map_err(|e| CodecError::Corrupt(format!("JP2 decode: {e}")))?;
        let full = raster_of(fmt, full_w, full_h, out);
        let copy_w = scaled.w.min(full_w.saturating_sub(scaled.x)).max(1);
        let copy_h = scaled.h.min(full_h.saturating_sub(scaled.y)).max(1);
        let mut cropped = full.zeroed_like(copy_w, copy_h)?;
        cropped.blit(
            &full,
            crate::image::CopyRect {
                src_x: scaled.x,
                src_y: scaled.y,
                width: copy_w,
                height: copy_h,
            },
            0,
            0,
        )?;
        Ok(cropped)
    }
}
