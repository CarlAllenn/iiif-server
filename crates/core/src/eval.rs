//! Request evaluation: a parsed [`ImageRequest`] plus concrete image
//! dimensions and deployment limits become an executable [`Plan`] — or a
//! spec-mandated error.
//!
//! All the dimension-dependent rules the grammar could not check live here.

use std::fmt;

use num_traits::cast::ToPrimitive;

use crate::{
    grammar::{ImageRequest, Quality, Region, Rotation, Size, SizeKind},
    info::Limits,
};

/// Round a non-negative float to `u32`, saturating at the type's ceiling.
/// Saturated values are always caught by the bounds/limits checks that
/// follow every call site — saturation just keeps the arithmetic total.
fn round_u32(v: f64) -> u32 {
    v.round().to_u32().unwrap_or(u32::MAX)
}

/// Floor variant of [`round_u32`].
fn floor_u32(v: f64) -> u32 {
    v.floor().to_u32().unwrap_or(u32::MAX)
}

/// The extracted region in full-resolution pixel coordinates, already
/// clipped to the image edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    /// Left edge in full-resolution pixels.
    pub x: u32,
    /// Top edge in full-resolution pixels.
    pub y: u32,
    /// Width in full-resolution pixels.
    pub w: u32,
    /// Height in full-resolution pixels.
    pub h: u32,
}

/// A fully evaluated request: everything the pipeline needs, nothing the
/// client sent left uninterpreted.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Region to extract, clipped, in full-resolution pixels.
    pub crop: CropRect,
    /// Output dimensions after scaling (before rotation swaps them).
    pub out_w: u32,
    /// Output height after scaling (before rotation swaps them).
    pub out_h: u32,
    /// Mirror before rotation.
    pub mirror: bool,
    /// Clockwise degrees, normalized to `0.0..360.0`.
    pub degrees: f64,
    /// Requested quality, passed through to the raster stage.
    pub quality: Quality,
    /// Requested output format, passed through to the encode stage.
    pub format: crate::grammar::Format,
    /// Whether the scale step upscales beyond the extracted region — used
    /// for the canonical `^` spelling.
    pub upscales: bool,
    /// Whether the size parameter was a `max` form — canonical spelling
    /// keeps `max` rather than `w,h`.
    size_was_max: bool,
    /// Full image dimensions, kept for canonical-form decisions.
    full_w: u32,
    full_h: u32,
}

/// Spec-mandated evaluation failures and their HTTP statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalError {
    /// Region entirely outside the image, or zero-pixel after clipping —
    /// 400.
    RegionOutOfBounds,
    /// Non-`^` size larger than the extracted region — 400.
    UpscaleWithoutFlag,
    /// Scaled dimensions below 1 pixel — 400.
    BelowOnePixel,
    /// Scaled dimensions above `maxWidth`/`maxHeight`/`maxArea` — 400.
    ExceedsLimits,
}

impl EvalError {
    /// The HTTP status the spec assigns to every evaluation failure.
    pub const HTTP_STATUS: u16 = 400;
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::RegionOutOfBounds => "region is outside the image bounds",
            Self::UpscaleWithoutFlag => {
                "requested size exceeds the extracted region (use the ^ prefix)"
            },
            Self::BelowOnePixel => "scaled size is below one pixel",
            Self::ExceedsLimits => "scaled size exceeds the published limits",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for EvalError {}

/// Evaluate a request against an image's full dimensions and the
/// deployment limits.
///
/// # Errors
///
/// Every [`EvalError`] maps to HTTP 400 per the spec's region/size rules.
pub fn evaluate(
    request: &ImageRequest,
    full_w: u32,
    full_h: u32,
    limits: Limits,
) -> Result<Plan, EvalError> {
    let crop = resolve_region(request.region, full_w, full_h)?;
    let (out_w, out_h, upscales) = resolve_size(request.size, crop.w, crop.h, limits)?;
    if out_w == 0 || out_h == 0 {
        return Err(EvalError::BelowOnePixel);
    }
    if out_w > limits.width
        || out_h > limits.height
        || u64::from(out_w) * u64::from(out_h) > limits.area
    {
        return Err(EvalError::ExceedsLimits);
    }
    Ok(Plan {
        crop,
        out_w,
        out_h,
        mirror: request.rotation.mirror,
        degrees: request.rotation.degrees % 360.0,
        quality: request.quality,
        format: request.format,
        upscales,
        size_was_max: matches!(request.size.kind, SizeKind::Max),
        full_w,
        full_h,
    })
}

fn resolve_region(region: Region, full_w: u32, full_h: u32) -> Result<CropRect, EvalError> {
    match region {
        Region::Full => Ok(CropRect {
            x: 0,
            y: 0,
            w: full_w,
            h: full_h,
        }),
        Region::Square => {
            let side = full_w.min(full_h);
            Ok(CropRect {
                x: (full_w - side) / 2,
                y: (full_h - side) / 2,
                w: side,
                h: side,
            })
        },
        Region::Pixels { x, y, w, h } => {
            if x >= full_w || y >= full_h {
                return Err(EvalError::RegionOutOfBounds);
            }
            Ok(CropRect {
                x,
                y,
                w: w.min(full_w - x),
                h: h.min(full_h - y),
            })
        },
        Region::Percent { x, y, w, h } => {
            let px = round_u32(x / 100.0 * f64::from(full_w));
            let py = round_u32(y / 100.0 * f64::from(full_h));
            if px >= full_w || py >= full_h {
                return Err(EvalError::RegionOutOfBounds);
            }
            let pw = round_u32(w / 100.0 * f64::from(full_w));
            let ph = round_u32(h / 100.0 * f64::from(full_h));
            if pw == 0 || ph == 0 {
                return Err(EvalError::RegionOutOfBounds);
            }
            Ok(CropRect {
                x: px,
                y: py,
                w: pw.min(full_w - px),
                h: ph.min(full_h - py),
            })
        },
    }
}

/// Returns `(out_w, out_h, upscales)`.
fn resolve_size(
    size: Size,
    region_w: u32,
    region_h: u32,
    limits: Limits,
) -> Result<(u32, u32, bool), EvalError> {
    let rw = f64::from(region_w);
    let rh = f64::from(region_h);
    let (out_w, out_h) = match size.kind {
        SizeKind::Max => {
            let fit = limit_fit_scale(rw, rh, limits);
            let scale = if size.upscale { fit } else { fit.min(1.0) };
            // Floor under the cap so rounding can never push past a limit.
            (
                floor_u32((rw * scale).max(1.0)),
                floor_u32((rh * scale).max(1.0)),
            )
        },
        SizeKind::Width(w) => {
            if !size.upscale && w > region_w {
                return Err(EvalError::UpscaleWithoutFlag);
            }
            let scale = f64::from(w) / rw;
            (w, round_u32(rh * scale))
        },
        SizeKind::Height(h) => {
            if !size.upscale && h > region_h {
                return Err(EvalError::UpscaleWithoutFlag);
            }
            let scale = f64::from(h) / rh;
            (round_u32(rw * scale), h)
        },
        SizeKind::Percent(pct) => {
            let scale = pct / 100.0;
            (round_u32(rw * scale), round_u32(rh * scale))
        },
        SizeKind::WidthHeight(w, h) => {
            if !size.upscale && (w > region_w || h > region_h) {
                return Err(EvalError::UpscaleWithoutFlag);
            }
            (w, h)
        },
        SizeKind::Confined(w, h) => {
            let fit = (f64::from(w) / rw).min(f64::from(h) / rh);
            // A confining box strictly larger than the region can only be
            // satisfied "as large as possible" by upscaling; without the
            // `^` flag the official validator requires a 400 here.
            if !size.upscale && fit > 1.0 {
                return Err(EvalError::UpscaleWithoutFlag);
            }
            (round_u32(rw * fit), round_u32(rh * fit))
        },
    };
    let upscales = out_w > region_w || out_h > region_h;
    if upscales && !size.upscale {
        // Belt and braces: rounding in the aspect-preserving forms can
        // nudge one dimension past the region; the spec calls that an
        // error only when the *requested* size exceeds the region, so
        // clamp instead of failing.
        return Ok((out_w.min(region_w), out_h.min(region_h), false));
    }
    Ok((out_w, out_h, upscales))
}

/// The largest scale of `rw`×`rh` that stays inside every limit.
fn limit_fit_scale(rw: f64, rh: f64, limits: Limits) -> f64 {
    let by_width = f64::from(limits.width) / rw;
    let by_height = f64::from(limits.height) / rh;
    let by_area = (limits.area.to_f64().unwrap_or(f64::MAX) / (rw * rh)).sqrt();
    by_width.min(by_height).min(by_area)
}

impl Plan {
    /// Whether the extracted region is the entire image.
    #[must_use]
    pub const fn is_full_region(&self) -> bool {
        self.crop.x == 0
            && self.crop.y == 0
            && self.crop.w == self.full_w
            && self.crop.h == self.full_h
    }

    /// The canonical request path (region/size/rotation/quality.format)
    /// per the spec's canonical-form rules, used for the `Link
    /// rel="canonical"` header.
    #[must_use]
    pub fn canonical_path(&self) -> String {
        let region = if self.is_full_region() {
            "full".to_owned()
        } else {
            format!(
                "{},{},{},{}",
                self.crop.x, self.crop.y, self.crop.w, self.crop.h
            )
        };
        let size = match (self.size_was_max, self.upscales) {
            (true, false) => "max".to_owned(),
            (true, true) => "^max".to_owned(),
            (false, false) => format!("{},{}", self.out_w, self.out_h),
            (false, true) => format!("^{},{}", self.out_w, self.out_h),
        };
        let rotation = Rotation {
            mirror: self.mirror,
            degrees: self.degrees,
        };
        format!(
            "{region}/{size}/{rotation}/{}.{}",
            self.quality, self.format
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test code: a panic here is the failure signal, not a crash path"
    )]

    use super::*;
    use crate::grammar::Format;

    const LIMITS: Limits = Limits {
        width: 10_000,
        height: 10_000,
        area: 100_000_000,
    };

    fn req(path: &str) -> ImageRequest {
        ImageRequest::parse(path).unwrap()
    }

    fn eval(path: &str, w: u32, h: u32) -> Result<Plan, EvalError> {
        evaluate(&req(path), w, h, LIMITS)
    }

    #[test]
    fn full_max_is_identity_under_limits() {
        let plan = eval("full/max/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!(
            plan.crop,
            CropRect {
                x: 0,
                y: 0,
                w: 6000,
                h: 4000
            }
        );
        assert_eq!((plan.out_w, plan.out_h), (6000, 4000));
        assert!(!plan.upscales);
        assert_eq!(plan.canonical_path(), "full/max/0/default.jpg");
    }

    #[test]
    fn max_respects_area_limit() {
        // 20k×20k image: maxArea 100M forces scale sqrt(100e6/400e6)=0.5.
        let limits = Limits {
            width: 20_000,
            height: 20_000,
            area: 100_000_000,
        };
        let plan = evaluate(&req("full/max/0/default.jpg"), 20_000, 20_000, limits).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (10_000, 10_000));
    }

    #[test]
    fn square_centers() {
        let plan = eval("square/max/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!(
            plan.crop,
            CropRect {
                x: 1000,
                y: 0,
                w: 4000,
                h: 4000
            }
        );
    }

    #[test]
    fn region_clips_at_edges() {
        let plan = eval("5000,3000,2000,2000/max/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!(
            plan.crop,
            CropRect {
                x: 5000,
                y: 3000,
                w: 1000,
                h: 1000
            }
        );
        // Canonical region is the clipped rectangle.
        assert!(plan.canonical_path().starts_with("5000,3000,1000,1000/"));
    }

    #[test]
    fn region_outside_is_400() {
        assert_eq!(
            eval("6000,0,10,10/max/0/default.jpg", 6000, 4000).unwrap_err(),
            EvalError::RegionOutOfBounds
        );
        assert_eq!(
            eval("0,4000,10,10/max/0/default.jpg", 6000, 4000).unwrap_err(),
            EvalError::RegionOutOfBounds
        );
        // pct x/y at or past 100 is rejected at parse time (grammar), so
        // the closest evaluation-level case is a tiny region that rounds
        // to zero pixels:
        assert_eq!(
            eval("pct:99.999,0,0.001,1/max/0/default.jpg", 60, 40).unwrap_err(),
            EvalError::RegionOutOfBounds
        );
    }

    #[test]
    fn percent_region_resolves() {
        let plan = eval("pct:25,25,50,50/max/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!(
            plan.crop,
            CropRect {
                x: 1500,
                y: 1000,
                w: 3000,
                h: 2000
            }
        );
    }

    #[test]
    fn width_scales_aspect() {
        let plan = eval("full/300,/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (300, 200));
        assert_eq!(plan.canonical_path(), "full/300,200/0/default.jpg");
    }

    #[test]
    fn upscale_without_flag_is_400() {
        assert_eq!(
            eval("full/7000,/0/default.jpg", 6000, 4000).unwrap_err(),
            EvalError::UpscaleWithoutFlag
        );
        assert_eq!(
            eval("full/7000,5000/0/default.jpg", 6000, 4000).unwrap_err(),
            EvalError::UpscaleWithoutFlag
        );
    }

    #[test]
    fn upscale_with_flag_works() {
        let plan = eval("full/^7500,/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (7500, 5000));
        assert!(plan.upscales);
        assert_eq!(plan.canonical_path(), "full/^7500,5000/0/default.jpg");
    }

    #[test]
    fn upscale_beyond_limits_is_400() {
        assert_eq!(
            eval("full/^20000,/0/default.jpg", 6000, 4000).unwrap_err(),
            EvalError::ExceedsLimits
        );
    }

    #[test]
    fn caret_max_scales_to_limits() {
        let plan = eval("full/^max/0/default.jpg", 500, 250).unwrap();
        // Fit: width 10000/500=20, height 10000/250=40, area sqrt(1e8/125e3)≈28.28 →
        // 20.
        assert_eq!((plan.out_w, plan.out_h), (10_000, 5_000));
        assert!(plan.upscales);
        assert_eq!(plan.canonical_path(), "full/^max/0/default.jpg");
    }

    #[test]
    fn confined_fits_inside_box() {
        let plan = eval("full/!300,300/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (300, 200));
        // A confining box larger than the region needs `^` (validator
        // rule): without it, 400.
        assert_eq!(
            eval("full/!9000,9000/0/default.jpg", 6000, 4000).unwrap_err(),
            EvalError::UpscaleWithoutFlag
        );
        // With ^ it scales up to the box.
        let plan = eval("full/^!9000,9000/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (9000, 6000));
    }

    #[test]
    fn percent_size() {
        let plan = eval("full/pct:50/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (3000, 2000));
        let plan = eval("full/^pct:150/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (9000, 6000));
    }

    #[test]
    fn below_one_pixel_is_400() {
        assert_eq!(
            eval("full/pct:0.001/0/default.jpg", 600, 400).unwrap_err(),
            EvalError::BelowOnePixel
        );
    }

    #[test]
    fn distorted_wh_allowed() {
        let plan = eval("full/300,300/0/default.jpg", 6000, 4000).unwrap();
        assert_eq!((plan.out_w, plan.out_h), (300, 300));
    }

    #[test]
    fn rotation_normalized() {
        let plan = eval("full/max/360/default.jpg", 600, 400).unwrap();
        assert_eq!(plan.degrees.to_bits(), 0.0_f64.to_bits());
        assert_eq!(plan.canonical_path(), "full/max/0/default.jpg");
        let plan = eval("full/max/!90/default.jpg", 600, 400).unwrap();
        assert!(plan.mirror);
        assert_eq!(plan.canonical_path(), "full/max/!90/default.jpg");
    }

    #[test]
    fn canonical_keeps_quality_and_format() {
        let plan = eval("square/!100,100/270/bitonal.png", 6000, 4000).unwrap();
        assert_eq!(plan.format, Format::Png);
        assert_eq!(
            plan.canonical_path(),
            "1000,0,4000,4000/100,100/270/bitonal.png"
        );
    }
}
