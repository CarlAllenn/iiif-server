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
    /// Scaled full-image width in pixels.
    pub width: u32,
    /// Scaled full-image height in pixels.
    pub height: u32,
}

/// One entry in `tiles`: a tile size plus the scale factors at which that
/// tiling is natively cheap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TileSet {
    /// Tile width in pixels.
    pub width: u32,
    /// Tile height when it differs from the width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Scale factors at which this tiling is natively cheap.
    #[serde(rename = "scaleFactors")]
    pub scale_factors: Vec<u32>,
}

/// Deployment-level size limits — the denial-of-service posture. Always
/// published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum output width in pixels.
    pub width: u32,
    /// Maximum output height in pixels.
    pub height: u32,
    /// Maximum output area in pixels (width × height).
    pub area: u64,
}

/// Everything the info.json needs about one image.
///
/// Its dimensions and the pyramid structure actually present in the master
/// (used to derive `tiles` and `sizes` so viewers request only
/// natively-cheap tiles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDescription {
    /// Full-resolution image width in pixels.
    pub width: u32,
    /// Full-resolution image height in pixels.
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
    /// The Image API 3.0 context URI.
    #[serde(rename = "@context")]
    pub context: &'static str,
    /// The image's canonical base URI.
    pub id: String,
    /// Always `"ImageService3"`.
    #[serde(rename = "type")]
    pub type_: &'static str,
    /// Always the Image API protocol URI.
    pub protocol: &'static str,
    /// Compliance level URI (level 2).
    pub profile: &'static str,
    /// Full-resolution width in pixels.
    pub width: u32,
    /// Full-resolution height in pixels.
    pub height: u32,
    /// Maximum output width the deployment serves.
    #[serde(rename = "maxWidth")]
    pub max_width: u32,
    /// Maximum output height the deployment serves.
    #[serde(rename = "maxHeight")]
    pub max_height: u32,
    /// Maximum output area the deployment serves.
    #[serde(rename = "maxArea")]
    pub max_area: u64,
    /// Complete scaled sizes (from the pyramid), smallest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<SizeEntry>,
    /// Native tilings and their scale factors.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<TileSet>,
    /// Qualities supported beyond the level-2 requirement.
    #[serde(rename = "extraQualities")]
    pub extra_qualities: &'static [&'static str],
    /// Formats supported beyond the level-2 requirement.
    #[serde(rename = "extraFormats")]
    pub extra_formats: &'static [&'static str],
    /// Feature names supported beyond the level-2 requirement.
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
            max_width: limits.width,
            max_height: limits.height,
            max_area: limits.area,
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
        #[allow(
            clippy::expect_used,
            reason = "map-free static shape: to_string cannot fail"
        )]
        serde_json::to_string(self).expect("info.json serialization is infallible")
    }
}
