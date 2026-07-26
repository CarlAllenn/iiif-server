//! The Image Information document (info.json), Image API 3.0 §5.
//!
//! Capability is baked in, not toggled: the only inputs here are the image
//! itself (dimensions, pyramid structure) and the deployment's numeric
//! limits. Everything else — profile, qualities, formats, features — is a
//! compile-time fact of the binary, identical for every image.

use serde::Serialize;

/// The v3 `@context` URI.
pub const CONTEXT: &str = "http://iiif.io/api/image/3/context.json";
/// The protocol URI, fixed by the spec.
pub const PROTOCOL: &str = "http://iiif.io/api/image";

/// One entry in `sizes`: a complete scaled version of the full image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SizeEntry {
    pub width: u32,
    pub height: u32,
}

/// One entry in `tiles`: a tile size plus the scale factors at which that
/// tiling is natively cheap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TileSet {
    pub width: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(rename = "scaleFactors")]
    pub scale_factors: Vec<u32>,
}

/// Deployment-level size limits — the denial-of-service posture. Always
/// published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_area: u64,
}

/// Everything the info.json needs about one image: its dimensions and the
/// pyramid structure actually present in the master (used to derive
/// `tiles` and `sizes` so viewers request only natively-cheap tiles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDescription {
    pub width: u32,
    pub height: u32,
    /// Tile dimensions and pyramid scale factors derived from the master's
    /// actual structure; empty for untiled sources.
    pub tiles: Vec<TileSet>,
    /// Complete scaled sizes derived from the pyramid levels.
    pub sizes: Vec<SizeEntry>,
}

/// The serialized info.json document.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    #[serde(rename = "@context")]
    pub context: &'static str,
    pub id: String,
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub protocol: &'static str,
    pub profile: &'static str,
    pub width: u32,
    pub height: u32,
    #[serde(rename = "maxWidth")]
    pub max_width: u32,
    #[serde(rename = "maxHeight")]
    pub max_height: u32,
    #[serde(rename = "maxArea")]
    pub max_area: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<SizeEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<TileSet>,
    #[serde(rename = "extraQualities")]
    pub extra_qualities: &'static [&'static str],
    #[serde(rename = "extraFormats")]
    pub extra_formats: &'static [&'static str],
    #[serde(rename = "extraFeatures")]
    pub extra_features: &'static [&'static str],
}

/// Qualities beyond the level-2 requirement that this binary always
/// supports. Level 2 requires `default`, `color` (if the image has color),
/// `gray`, `bitonal`; we publish the full set explicitly.
pub const EXTRA_QUALITIES: &[&str] = &["color", "gray", "bitonal"];

/// Formats beyond the level-2 requirement (`jpg`, `png`) the binary
/// encodes — the complete spec table (webp is lossless-only, the one
/// documented asterisk). Never lies.
pub const EXTRA_FORMATS: &[&str] = &["gif", "jp2", "pdf", "tif", "webp"];

/// Feature names beyond the level-2 set that this binary supports today,
/// from the v3 feature-name table. Grows as milestones land; never lies.
pub const EXTRA_FEATURES: &[&str] = &[
    "mirroring",
    "regionSquare",
    "rotationArbitrary",
    "sizeByConfinedWh",
    "sizeByDistortedWh",
    "sizeByWh",
    "sizeUpscaling",
];

impl Info {
    /// Assemble the document for one image. `id` is the full base URI of
    /// the image (scheme, server, prefix, identifier — no trailing slash).
    #[must_use]
    pub fn new(id: String, image: &ImageDescription, limits: Limits) -> Self {
        Self {
            context: CONTEXT,
            id,
            type_: "ImageService3",
            protocol: PROTOCOL,
            profile: "level2",
            width: image.width,
            height: image.height,
            max_width: limits.max_width,
            max_height: limits.max_height,
            max_area: limits.max_area,
            sizes: image.sizes.clone(),
            tiles: image.tiles.clone(),
            extra_qualities: EXTRA_QUALITIES,
            extra_formats: EXTRA_FORMATS,
            extra_features: EXTRA_FEATURES,
        }
    }

    /// Serialize to the wire form.
    ///
    /// # Panics
    ///
    /// Panics only if `serde_json` breaks its own contract: serialization
    /// of this struct is structurally infallible (no maps, no non-string
    /// keys, no fallible `Serialize` impls).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("info.json serialization is infallible")
    }
}
