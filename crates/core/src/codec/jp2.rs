//! JP2 / HTJ2K masters via the pure-Rust `j2k` crate — the capability no
//! `OpenJPEG`-based incumbent has, validated bit-exact against `OpenJPEG` by
//! SPIKE 2.

use j2k::{CpuDecodeParallelism, J2kDecoder, J2kScratchPool, PixelFormat, Rect};

use super::{CodecError, Master};
use crate::{
    eval::CropRect,
    image::Raster,
    info::{ImageDescription, SizeEntry, TileSet},
};

/// Wrap raw interleaved samples in the right raster variant.
const fn raster_of(fmt: PixelFormat, width: u32, height: u32, data: Vec<u8>) -> Raster {
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

/// An opened JP2/HTJ2K master.
///
/// Owns the compressed bytes; decoders borrow them per request (parse state is
/// cheap relative to pixel work, and a fresh decoder per decode keeps the type
/// `Send` for the worker pool).
pub struct Jp2Master {
    bytes: Vec<u8>,
    // (Debug impl below skips `bytes` — megabytes of codestream.)
    width: u32,
    height: u32,
    components: u16,
    resolution_levels: u8,
    tile: (u32, u32),
    /// Live pool-pressure hint; see `Master::set_internal_parallelism`.
    internal_parallelism: bool,
}

impl core::fmt::Debug for Jp2Master {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Jp2Master")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("components", &self.components)
            .field("resolution_levels", &self.resolution_levels)
            .field("tile", &self.tile)
            .finish_non_exhaustive()
    }
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
        Ok(Self {
            bytes,
            width,
            height,
            components,
            resolution_levels,
            tile,
            internal_parallelism: false,
        })
    }

    const fn pixel_format(&self) -> PixelFormat {
        if self.components == 1 {
            PixelFormat::Gray8
        } else {
            PixelFormat::Rgb8
        }
    }

    /// The deepest cheap downscale, as a resolution-halving count:
    /// bounded only by the codestream's own resolution ladder. The old
    /// 1/8 ceiling came from the shared `Downscale` enum (a DCT-geometry
    /// limit foreign to wavelets); `decode_region_scaled_pow2_into` walks
    /// the whole ladder, so a 165 MP master rendered at 512 px decodes at
    /// 1/16–1/32 instead of decoding 1/8 and resampling.
    fn levels_for(&self, needed: f64) -> u8 {
        let max_level = self.resolution_levels - 1;
        let mut choice = 0u8;
        for level in 1..=max_level {
            if f64::from(1u32 << u32::from(level).min(31)) <= needed {
                choice = level;
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

    fn set_internal_parallelism(&mut self, allow: bool) {
        self.internal_parallelism = allow;
    }

    fn advisories(&self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.resolution_levels <= 1 && u64::from(self.width) * u64::from(self.height) > 4_000_000
        {
            notes.push(
                "large JP2 with a single resolution level: zoomed-out requests decode the \
                full image. Re-encode with resolution levels, e.g.: opj_compress -n 6"
                    .to_owned(),
            );
        }
        notes
    }

    fn decode_crop(&mut self, crop: CropRect, needed: f64) -> Result<Raster, CodecError> {
        let mut decoder = J2kDecoder::new(&self.bytes)
            .map_err(|e| CodecError::Corrupt(format!("JP2 parse: {e}")))?;
        // Pool pressure decides: an idle pool wants the codec's internal
        // parallelism (1.7× lower latency), a saturated one does not
        // (oversubscription costs ~16% throughput). See
        // `Master::set_internal_parallelism`.
        decoder.set_cpu_decode_parallelism(if self.internal_parallelism {
            CpuDecodeParallelism::Auto
        } else {
            CpuDecodeParallelism::Serial
        });
        let levels = self.levels_for(needed);
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
        let scaled = roi.scaled_covering_denominator(1u32 << u32::from(levels).min(31));
        let mut pool = J2kScratchPool::new();
        let stride = scaled.w as usize * bpp;
        let mut out = vec![0u8; stride * scaled.h as usize];
        decoder
            .decode_region_scaled_pow2_into(&mut pool, &mut out, stride, fmt, roi, levels)
            .map_err(|e| CodecError::Corrupt(format!("JP2 decode: {e}")))?;
        Ok(raster_of(fmt, scaled.w, scaled.h, out))
    }
}
