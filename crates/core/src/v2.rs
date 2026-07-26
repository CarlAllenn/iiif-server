//! Image API 2.1 translation layer: the full 18-feature v2.1 endpoint
//! expressed over the same engine (design spec: `full`↔`max` aliasing,
//! profile-array info.json, `@id` vs `id`, `sizeAboveFull` mapped to the
//! upscale path, `sizeByDistortedWh` as non-aspect `w,h`).
//!
//! v2 requests parse into the same [`ImageRequest`] the engine evaluates;
//! only the size grammar and the document/canonical spellings differ.

use crate::eval::Plan;
use crate::grammar::{
    Component, Format, ImageRequest, ParseError, Quality, Region, Rotation, Size, SizeKind,
};
use crate::info::{ImageDescription, Limits};

/// A parsed v2.1 request plus what the v2 canonical form needs to
/// remember about the original size spelling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct V2Request {
    pub request: ImageRequest,
    /// v2 canonical size is `w,` for aspect-preserving forms, `w,h` for
    /// the distorted form, `full` for the full-size aliases.
    pub aspect_preserved: bool,
    pub was_full: bool,
}

/// Parse the v2.1 `{region}/{size}/{rotation}/{quality}.{format}` suffix.
///
/// Differences from v3, per the 2.1 spec: `full` (and 2.1's `max`) name
/// the full/maximal size; there is no `^` prefix — `sizeAboveFull` means
/// every numeric form may upscale freely (bounded by the published
/// limits, enforced at evaluation).
///
/// # Errors
///
/// [`ParseError`] (HTTP 400) exactly like the v3 grammar.
pub fn parse_image_request(path: &str) -> Result<V2Request, ParseError> {
    let structure = || ParseError {
        component: Component::Structure,
        input: path.to_owned(),
    };
    let mut segments = path.split('/');
    let (region, size, rotation, last) = (
        segments.next().ok_or_else(structure)?,
        segments.next().ok_or_else(structure)?,
        segments.next().ok_or_else(structure)?,
        segments.next().ok_or_else(structure)?,
    );
    if segments.next().is_some() {
        return Err(structure());
    }
    let (quality, format) = last.rsplit_once('.').ok_or_else(structure)?;
    let (size, aspect_preserved, was_full) = parse_size(size)?;
    Ok(V2Request {
        request: ImageRequest {
            region: Region::parse(region)?,
            size,
            rotation: Rotation::parse(rotation)?,
            quality: Quality::parse(quality)?,
            format: Format::parse(format)?,
        },
        aspect_preserved,
        was_full,
    })
}

/// Returns `(size, aspect_preserved, was_full)`.
fn parse_size(input: &str) -> Result<(Size, bool, bool), ParseError> {
    let err = || ParseError {
        component: Component::Size,
        input: input.to_owned(),
    };
    // v2 has no `^` prefix at all.
    if input.starts_with('^') {
        return Err(err());
    }
    if input == "full" || input == "max" {
        return Ok((
            Size {
                upscale: false,
                kind: SizeKind::Max,
            },
            true,
            true,
        ));
    }
    // Numeric forms: reuse the v3 component grammar, then lift the
    // upscale restriction (`sizeAboveFull`). `pct:n` above 100 must
    // bypass the v3 parse-time cap the same way.
    let parsed = if let Some(pct) = input.strip_prefix("pct:") {
        let spelled = format!("^pct:{pct}");
        Size::parse(&spelled).map_err(|_| err())?
    } else {
        Size::parse(input).map_err(|_| err())?
    };
    let aspect = !matches!(parsed.kind, SizeKind::WidthHeight(..));
    Ok((
        Size {
            upscale: true,
            kind: parsed.kind,
        },
        aspect,
        false,
    ))
}

/// The v2 canonical request path (§4.7 of the 2.1 spec): region `full` or
/// pixels; size `full`, `w,` (aspect preserved) or `w,h`; rotation and
/// quality as literals.
#[must_use]
pub fn canonical_path(plan: &Plan, v2: &V2Request) -> String {
    let full_region = plan.is_full_region();
    let region = if full_region {
        "full".to_owned()
    } else {
        format!(
            "{},{},{},{}",
            plan.crop.x, plan.crop.y, plan.crop.w, plan.crop.h
        )
    };
    let size = if v2.was_full && full_region && !plan.upscales {
        "full".to_owned()
    } else if v2.aspect_preserved {
        format!("{},", plan.out_w)
    } else {
        format!("{},{}", plan.out_w, plan.out_h)
    };
    let rotation = Rotation {
        mirror: plan.mirror,
        degrees: plan.degrees,
    };
    format!(
        "{region}/{size}/{rotation}/{}.{}",
        plan.quality, plan.format
    )
}

/// The v2 `@context` URI.
pub const CONTEXT_V2: &str = "http://iiif.io/api/image/2/context.json";
/// The v2 level-2 profile document URI.
pub const LEVEL2_V2: &str = "http://iiif.io/api/image/2/level2.json";

/// Named v2.1 features this binary supports beyond level 2, from the
/// official compliance document. Grows (rotationArbitrary) with the
/// completionist sweep; never lies.
pub const SUPPORTS_BEYOND_LEVEL2: &[&str] = &[
    "canonicalLinkHeader",
    "mirroring",
    "profileLinkHeader",
    "regionSquare",
    "sizeAboveFull",
];

/// Output formats the binary can encode today, published in the profile.
pub const FORMATS: &[&str] = &["jpg", "png"];
/// Qualities, all four, always.
pub const QUALITIES: &[&str] = &["default", "color", "gray", "bitonal"];

/// Build the v2 Image Information document.
///
/// # Panics
///
/// Panics only if `serde_json` breaks its own contract: the document is
/// a static shape with string keys throughout.
#[must_use]
pub fn info_json(id: &str, image: &ImageDescription, limits: Limits) -> String {
    let sizes: Vec<serde_json::Value> = image
        .sizes
        .iter()
        .map(|entry| serde_json::json!({"width": entry.width, "height": entry.height}))
        .collect();
    let tiles: Vec<serde_json::Value> = image
        .tiles
        .iter()
        .map(|tile| {
            let mut object = serde_json::json!({
                "width": tile.width,
                "scaleFactors": tile.scale_factors,
            });
            if let Some(height) = tile.height {
                object["height"] = height.into();
            }
            object
        })
        .collect();
    let mut document = serde_json::json!({
        "@context": CONTEXT_V2,
        "@id": id,
        "protocol": "http://iiif.io/api/image",
        "width": image.width,
        "height": image.height,
        "profile": [
            LEVEL2_V2,
            {
                "formats": FORMATS,
                "qualities": QUALITIES,
                "supports": SUPPORTS_BEYOND_LEVEL2,
                "maxWidth": limits.max_width,
                "maxHeight": limits.max_height,
                "maxArea": limits.max_area,
            }
        ],
    });
    if !sizes.is_empty() {
        document["sizes"] = sizes.into();
    }
    if !tiles.is_empty() {
        document["tiles"] = tiles.into();
    }
    serde_json::to_string(&document).expect("static shape")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::evaluate;

    const LIMITS: Limits = Limits {
        max_width: 4000,
        max_height: 4000,
        max_area: 16_000_000,
    };

    #[test]
    fn full_aliases_max() {
        let parsed = parse_image_request("full/full/0/default.jpg").unwrap();
        assert_eq!(parsed.request.size.kind, SizeKind::Max);
        let parsed = parse_image_request("full/max/0/default.jpg").unwrap();
        assert_eq!(parsed.request.size.kind, SizeKind::Max);
    }

    #[test]
    fn size_above_full_upscales_without_caret() {
        let parsed = parse_image_request("full/1500,/0/default.jpg").unwrap();
        let plan = evaluate(&parsed.request, 1000, 800, LIMITS).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (1500, 1200));
        // pct over 100 is legal in v2.
        let parsed = parse_image_request("full/pct:150/0/default.jpg").unwrap();
        let plan = evaluate(&parsed.request, 1000, 800, LIMITS).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (1500, 1200));
    }

    #[test]
    fn caret_is_not_v2() {
        assert!(parse_image_request("full/^max/0/default.jpg").is_err());
        assert!(parse_image_request("full/^150,/0/default.jpg").is_err());
    }

    #[test]
    fn distorted_wh_is_the_only_non_aspect_form() {
        let parsed = parse_image_request("full/300,300/0/default.jpg").unwrap();
        assert!(!parsed.aspect_preserved);
        let parsed = parse_image_request("full/!300,300/0/default.jpg").unwrap();
        assert!(parsed.aspect_preserved);
    }

    #[test]
    fn canonical_uses_v2_spellings() {
        let parsed = parse_image_request("full/400,/0/default.jpg").unwrap();
        let plan = evaluate(&parsed.request, 1000, 800, LIMITS).unwrap();
        assert_eq!(canonical_path(&plan, &parsed), "full/400,/0/default.jpg");

        let parsed = parse_image_request("100,100,300,300/300,300/90/gray.png").unwrap();
        let plan = evaluate(&parsed.request, 1000, 800, LIMITS).unwrap();
        assert_eq!(
            canonical_path(&plan, &parsed),
            "100,100,300,300/300,300/90/gray.png"
        );

        let parsed = parse_image_request("full/full/0/default.jpg").unwrap();
        let plan = evaluate(&parsed.request, 1000, 800, LIMITS).unwrap();
        assert_eq!(canonical_path(&plan, &parsed), "full/full/0/default.jpg");
    }

    #[test]
    fn info_document_shape() {
        let description = ImageDescription {
            width: 1024,
            height: 768,
            tiles: vec![crate::info::TileSet {
                width: 256,
                height: None,
                scale_factors: vec![1, 2, 4],
            }],
            sizes: vec![crate::info::SizeEntry {
                width: 1024,
                height: 768,
            }],
        };
        let json: serde_json::Value =
            serde_json::from_str(&info_json("https://x/iiif/2/a", &description, LIMITS)).unwrap();
        assert_eq!(json["@context"], CONTEXT_V2);
        assert_eq!(json["@id"], "https://x/iiif/2/a");
        assert_eq!(json["profile"][0], LEVEL2_V2);
        assert_eq!(json["profile"][1]["qualities"][2], "gray");
        assert!(
            json["profile"][1]["supports"]
                .as_array()
                .unwrap()
                .iter()
                .any(|feature| feature == "sizeAboveFull")
        );
        assert_eq!(json["tiles"][0]["width"], 256);
    }
}
