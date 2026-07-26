//! Codec layer: turns master files into [`Raster`] regions, behind the
//! [`Master`] trait (the codec seam the design spec requires — Plan B for
//! any one format is a contained swap).
//!
//! Every decoder is pure Rust (the zero-C-parsing-untrusted-input
//! property). Decoders are synchronous; the async source seam bridges at
//! the call boundary (see `iiif-sources`).
//!
//! Formats: pyramidal/tiled TIFF (this file), JP2/HTJ2K ([`jp2`]), plain
//! JPEG and PNG ([`simple`]).

pub mod jp2;
pub mod simple;

use crate::eval::CropRect;
use crate::image::{Raster, RasterError};
use crate::info::{ImageDescription, SizeEntry, TileSet};
use num_traits::cast::ToPrimitive;
use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use tiff::ColorType;
use tiff::decoder::{Decoder, DecodingResult};

/// A master image opened for serving: enough metadata for info.json and
/// the ability to decode any clipped full-resolution crop at (at least)
/// the detail an output scale needs.
pub trait Master: Send {
    /// Full-resolution dimensions.
    fn dimensions(&self) -> (u32, u32);

    /// The info.json ingredients derived from the master's actual
    /// structure.
    fn describe(&self) -> ImageDescription;

    /// Decode `crop` (full-resolution coordinates, already clipped by the
    /// evaluation layer) with enough detail for a downscale factor of
    /// `needed` (full-res pixels per output pixel, ≥ 1). The result may be
    /// larger than `crop`/`needed` implies — the pipeline resamples to the
    /// exact output size.
    ///
    /// # Errors
    ///
    /// Decode failures; see [`CodecError`].
    fn decode_crop(&mut self, crop: CropRect, needed: f64) -> Result<Raster, CodecError>;
}

/// Sniff the container format and open the right decoder.
///
/// TIFF stays streaming (ranged reads through the source seam); the
/// JPEG-2000/JPEG/PNG paths read the remaining bytes — the design spec's
/// acknowledged model for JP2 (`&[u8]` input; bounded chunk caching is the
/// object-store refinement at M4).
///
/// # Errors
///
/// [`CodecError::Unsupported`] with an actionable message for anything
/// outside the supported matrix.
pub fn open_master<R: Read + Seek + Send + 'static>(
    mut reader: R,
) -> Result<Box<dyn Master>, CodecError> {
    let mut magic = [0u8; 12];
    let got = read_up_to(&mut reader, &mut magic)
        .map_err(|e| CodecError::Corrupt(format!("cannot read file header: {e}")))?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|e| CodecError::Corrupt(format!("cannot rewind: {e}")))?;
    let magic = &magic[..got];
    if magic.starts_with(b"II*\0") || magic.starts_with(b"MM\0*") {
        return Ok(Box::new(TiffPyramid::open(reader)?));
    }
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| CodecError::Corrupt(format!("cannot read master: {e}")))?;
    if (magic.len() >= 12 && &magic[4..12] == b"jP  \r\n\x87\n")
        || magic.starts_with(b"\xFF\x4F\xFF\x51")
    {
        return Ok(Box::new(jp2::Jp2Master::new(bytes)?));
    }
    if magic.starts_with(b"\xFF\xD8") {
        return Ok(Box::new(simple::SimpleMaster::from_jpeg(&bytes)?));
    }
    if magic.starts_with(b"\x89PNG") {
        return Ok(Box::new(simple::SimpleMaster::from_png(&bytes)?));
    }
    Err(CodecError::Unsupported(
        "unrecognized master format (supported: pyramidal TIFF, JP2/HTJ2K, JPEG, PNG)".to_owned(),
    ))
}

/// Read as many of `buf` as the reader will give without erroring on EOF.
fn read_up_to<R: Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// One resolution level of a pyramid, in its own pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelInfo {
    /// IFD index inside the TIFF.
    pub ifd: usize,
    pub width: u32,
    pub height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    /// Full-resolution pixels per pixel at this level (1, 2, 4, …).
    pub scale_factor: u32,
}

/// Codec-layer failure.
#[derive(Debug)]
pub enum CodecError {
    /// The master is outside the supported matrix — one actionable
    /// message, never a wrong image.
    Unsupported(String),
    /// The master is malformed.
    Corrupt(String),
    /// Pixel bookkeeping failed (internal bug).
    Raster(RasterError),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(msg) => write!(f, "unsupported master: {msg}"),
            Self::Corrupt(msg) => write!(f, "corrupt master: {msg}"),
            Self::Raster(e) => write!(f, "pixel bookkeeping: {e}"),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<RasterError> for CodecError {
    fn from(e: RasterError) -> Self {
        Self::Raster(e)
    }
}

impl From<tiff::TiffError> for CodecError {
    fn from(e: tiff::TiffError) -> Self {
        match e {
            tiff::TiffError::UnsupportedError(inner) => Self::Unsupported(inner.to_string()),
            other => Self::Corrupt(other.to_string()),
        }
    }
}

/// An opened pyramidal/tiled TIFF master.
pub struct TiffPyramid<R: Read + Seek> {
    decoder: Decoder<R>,
    levels: Vec<LevelInfo>,
    /// IFD index the decoder currently points at, to avoid useless seeks.
    current_ifd: usize,
}

impl<R: Read + Seek> TiffPyramid<R> {
    /// Open a TIFF and survey its pyramid structure.
    ///
    /// # Errors
    ///
    /// [`CodecError::Unsupported`] for masters outside the supported
    /// matrix (untiled, mixed layouts); [`CodecError::Corrupt`] for
    /// malformed files.
    pub fn open(reader: R) -> Result<Self, CodecError> {
        let mut decoder = Decoder::new(reader)?;
        let mut levels = Vec::new();
        let mut ifd = 0usize;
        let (full_w, full_h) = decoder.dimensions()?;
        loop {
            let (width, height) = decoder.dimensions()?;
            if decoder.get_chunk_type() != tiff::decoder::ChunkType::Tile {
                return Err(CodecError::Unsupported(format!(
                    "IFD {ifd} is not tiled; this master will serve slowly — \
                    convert with: vips tiffsave in.tif out.tif --tile --pyramid"
                )));
            }
            let (tile_width, tile_height) = decoder.chunk_dimensions();
            let scale_factor = (f64::from(full_w) / f64::from(width))
                .round()
                .to_u32()
                .unwrap_or(1);
            levels.push(LevelInfo {
                ifd,
                width,
                height,
                tile_width,
                tile_height,
                scale_factor: scale_factor.max(1),
            });
            if !decoder.more_images() {
                break;
            }
            decoder.next_image()?;
            ifd += 1;
        }
        // The pyramid contract: strictly descending levels. A same-size or
        // growing "level" means this is a multi-page document, not a
        // pyramid.
        for pair in levels.windows(2) {
            if pair[1].width >= pair[0].width || pair[1].height >= pair[0].height {
                return Err(CodecError::Unsupported(
                    "multiple full-size images (multi-page TIFF?), not a pyramid".to_owned(),
                ));
            }
        }
        let _ = full_h;
        Ok(Self {
            decoder,
            levels,
            current_ifd: ifd,
        })
    }

    /// Pyramid levels, largest first.
    #[must_use]
    pub fn levels(&self) -> &[LevelInfo] {
        &self.levels
    }

    /// Full-resolution dimensions.
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        (self.levels[0].width, self.levels[0].height)
    }

    /// The info.json ingredients derived from the actual pyramid: tile
    /// size with the real scale factors, and one `sizes` entry per level
    /// (ascending), so viewers request only natively-cheap tiles.
    #[must_use]
    pub fn describe(&self) -> ImageDescription {
        let base = &self.levels[0];
        let scale_factors: Vec<u32> = self.levels.iter().map(|l| l.scale_factor).collect();
        let tiles = vec![TileSet {
            width: base.tile_width,
            height: if base.tile_height == base.tile_width {
                None
            } else {
                Some(base.tile_height)
            },
            scale_factors,
        }];
        let mut sizes: Vec<SizeEntry> = self
            .levels
            .iter()
            .map(|l| SizeEntry {
                width: l.width,
                height: l.height,
            })
            .collect();
        sizes.reverse(); // ascending by width, per spec recommendation
        ImageDescription {
            width: base.width,
            height: base.height,
            tiles,
            sizes,
        }
    }

    /// Pick the smallest level that still has enough detail for a
    /// downscale factor of `needed` (full-res pixels per output pixel).
    #[must_use]
    pub fn level_for_scale(&self, needed: f64) -> &LevelInfo {
        self.levels
            .iter()
            .filter(|l| f64::from(l.scale_factor) <= needed.max(1.0))
            .max_by_key(|l| l.scale_factor)
            .unwrap_or(&self.levels[0])
    }

    /// Decode the axis-aligned region `(x, y, w, h)` — in *this level's*
    /// coordinates — by decoding exactly the tiles it touches.
    ///
    /// # Errors
    ///
    /// Propagates decode failures; rejects out-of-bounds requests as
    /// [`CodecError::Corrupt`] (they indicate a caller bug, not a client
    /// error — the evaluation layer already clipped).
    pub fn decode_region(
        &mut self,
        level_ifd: usize,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> Result<Raster, CodecError> {
        let level = *self
            .levels
            .iter()
            .find(|l| l.ifd == level_ifd)
            .ok_or_else(|| CodecError::Corrupt(format!("no level with IFD {level_ifd}")))?;
        if x.checked_add(w).is_none_or(|edge| edge > level.width)
            || y.checked_add(h).is_none_or(|edge| edge > level.height)
            || w == 0
            || h == 0
        {
            return Err(CodecError::Corrupt(
                "region outside level bounds".to_owned(),
            ));
        }
        if self.current_ifd != level.ifd {
            self.decoder.seek_to_image(level.ifd)?;
            self.current_ifd = level.ifd;
        }
        let tiles_across = level.width.div_ceil(level.tile_width);
        let first_col = x / level.tile_width;
        let last_col = (x + w - 1) / level.tile_width;
        let first_row = y / level.tile_height;
        let last_row = (y + h - 1) / level.tile_height;

        let mut out: Option<Raster> = None;
        for tile_row in first_row..=last_row {
            for tile_col in first_col..=last_col {
                let index = tile_row * tiles_across + tile_col;
                let tile = self.decode_tile(level, index)?;
                let out_buf = match &mut out {
                    Some(buf) => buf,
                    None => out.insert(tile.zeroed_like(w, h)?),
                };
                // Intersect this tile's footprint with the request.
                let tile_left = tile_col * level.tile_width;
                let tile_top = tile_row * level.tile_height;
                let left = x.max(tile_left);
                let top = y.max(tile_top);
                let right = (x + w).min(tile_left + tile.width());
                let bottom = (y + h).min(tile_top + tile.height());
                if right <= left || bottom <= top {
                    continue;
                }
                out_buf.blit(
                    &tile,
                    crate::image::CopyRect {
                        src_x: left - tile_left,
                        src_y: top - tile_top,
                        width: right - left,
                        height: bottom - top,
                    },
                    left - x,
                    top - y,
                )?;
            }
        }
        out.ok_or_else(|| CodecError::Corrupt("empty region decode".to_owned()))
    }

    /// Decode one tile to a raster. The tiff crate hands back
    /// edge-clipped dimensions for boundary tiles.
    fn decode_tile(&mut self, level: LevelInfo, index: u32) -> Result<Raster, CodecError> {
        let (data_w, data_h) = self.decoder.chunk_data_dimensions(index);
        let colortype = self.decoder.colortype()?;
        let result = self.decoder.read_chunk(index)?;
        raster_from_decoded(result, colortype, data_w, data_h, level)
    }
}

/// Convert the tiff crate's decode output into our raster model. M0
/// supports 8-bit gray and RGB; the M2 matrix widens this.
fn raster_from_decoded(
    result: DecodingResult,
    colortype: ColorType,
    width: u32,
    height: u32,
    level: LevelInfo,
) -> Result<Raster, CodecError> {
    let DecodingResult::U8(data) = result else {
        return Err(CodecError::Unsupported(format!(
            "sample format {colortype:?} not yet in the supported matrix"
        )));
    };
    let pixels = width as usize * height as usize;
    match colortype {
        ColorType::Gray(8) => {
            let mut data = data;
            data.truncate(pixels);
            if data.len() < pixels {
                return Err(CodecError::Corrupt("short tile data".to_owned()));
            }
            Ok(Raster::Gray8 {
                width,
                height,
                data,
            })
        }
        ColorType::RGB(8) => {
            let mut data = data;
            data.truncate(pixels * 3);
            if data.len() < pixels * 3 {
                return Err(CodecError::Corrupt("short tile data".to_owned()));
            }
            Ok(Raster::Rgb8 {
                width,
                height,
                data,
            })
        }
        ColorType::YCbCr(8) => {
            // For JPEG-compressed tiles the tiff crate keeps the JPEG
            // decoder's *input* colorspace, so the buffer holds
            // interleaved, already-upsampled Y′CbCr samples — the
            // conversion to RGB is ours (JPEG full-range BT.601). SPIKE 1
            // caught exactly this: treating these samples as RGB produced
            // a mean channel error of 89/255 against the libjpeg golden.
            let mut data = data;
            data.truncate(pixels * 3);
            if data.len() < pixels * 3 {
                return Err(CodecError::Corrupt("short tile data".to_owned()));
            }
            for px in data.chunks_exact_mut(3) {
                let [y, cb, cr] = [f64::from(px[0]), f64::from(px[1]), f64::from(px[2])];
                let r = y + 1.402 * (cr - 128.0);
                let g = y - 0.344_136 * (cb - 128.0) - 0.714_136 * (cr - 128.0);
                let b = y + 1.772 * (cb - 128.0);
                px[0] = r.round().clamp(0.0, 255.0).to_u8().unwrap_or(0);
                px[1] = g.round().clamp(0.0, 255.0).to_u8().unwrap_or(0);
                px[2] = b.round().clamp(0.0, 255.0).to_u8().unwrap_or(0);
            }
            Ok(Raster::Rgb8 {
                width,
                height,
                data,
            })
        }
        other => Err(CodecError::Unsupported(format!(
            "color type {other:?} not yet in the supported matrix \
            (level {}×{})",
            level.width, level.height
        ))),
    }
}

impl<R: Read + Seek + Send> Master for TiffPyramid<R> {
    fn dimensions(&self) -> (u32, u32) {
        Self::dimensions(self)
    }

    fn describe(&self) -> ImageDescription {
        Self::describe(self)
    }

    fn decode_crop(&mut self, crop: CropRect, needed: f64) -> Result<Raster, CodecError> {
        // Pick the pyramid level with just enough detail, then map the
        // full-resolution crop into that level's coordinates.
        let level = *self.level_for_scale(needed);
        let factor = f64::from(level.scale_factor);
        let left = ((f64::from(crop.x) / factor).floor())
            .to_u32()
            .unwrap_or(u32::MAX)
            .min(level.width.saturating_sub(1));
        let top = ((f64::from(crop.y) / factor).floor())
            .to_u32()
            .unwrap_or(u32::MAX)
            .min(level.height.saturating_sub(1));
        let right = ((f64::from(crop.x) + f64::from(crop.w)) / factor)
            .ceil()
            .to_u32()
            .unwrap_or(u32::MAX)
            .min(level.width);
        let bottom = ((f64::from(crop.y) + f64::from(crop.h)) / factor)
            .ceil()
            .to_u32()
            .unwrap_or(u32::MAX)
            .min(level.height);
        let region_w = (right - left).max(1);
        let region_h = (bottom - top).max(1);
        self.decode_region(level.ifd, left, top, region_w, region_h)
    }
}
