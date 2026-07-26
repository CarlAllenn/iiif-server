//! Typed IIIF Image API 3.0 request grammar.
//!
//! Parsing is strict: every accepted string is spec-legal, every value is
//! range-checked at parse time where the check needs no image dimensions.
//! Checks that need the image (region clipping, non-`^` size overflow,
//! server limits) happen at evaluation, not here.
//!
//! `Display` prints the *literal* canonical spelling of a value (integers
//! without leading zeros, floats per the spec's float-formatting rules).
//! Canonicalization against a concrete image (region → pixels, size →
//! `w,h`) is a separate evaluation-layer concern.

use std::fmt;

/// The image region to extract, per §4.1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Region {
    /// `full` — the entire image.
    Full,
    /// `square` — centered square, side = the shorter image dimension.
    Square,
    /// `x,y,w,h` — pixel coordinates. `w`, `h` are non-zero (parse-time
    /// rule: zero width or height is always a 400).
    Pixels { x: u32, y: u32, w: u32, h: u32 },
    /// `pct:x,y,w,h` — percentages of full-image dimensions.
    Percent { x: f64, y: f64, w: f64, h: f64 },
}

/// How the scaled dimensions are derived, per §4.2 (without the `^` flag,
/// which lives on [`Size`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeKind {
    /// `max` — maximum size available under server limits.
    Max,
    /// `w,` — exact width, aspect preserved.
    Width(u32),
    /// `,h` — exact height, aspect preserved.
    Height(u32),
    /// `pct:n` — scale by percentage of the extracted region.
    Percent(f64),
    /// `w,h` — exact dimensions, aspect NOT preserved.
    WidthHeight(u32, u32),
    /// `!w,h` — best fit inside `w`×`h`, aspect preserved.
    Confined(u32, u32),
}

/// The size parameter: an optional `^` upscale flag plus a [`SizeKind`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    /// `^` prefix: upscaling beyond the extracted region is permitted.
    pub upscale: bool,
    pub kind: SizeKind,
}

/// The rotation parameter, per §4.3: optional mirror, then clockwise
/// degrees in `0..=360`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    /// `!` prefix: mirror on the vertical axis before rotating.
    pub mirror: bool,
    pub degrees: f64,
}

/// The quality parameter, per §4.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quality {
    Default,
    Color,
    Gray,
    Bitonal,
}

/// The format parameter, per §4.5. All spec-enumerated formats parse; which
/// ones the server can *encode* is a capability question answered
/// elsewhere (unsupported-but-well-formed → 400 per spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Jpg,
    Tif,
    Png,
    Gif,
    Jp2,
    Pdf,
    Webp,
}

/// A complete parsed image request: `{region}/{size}/{rotation}/{quality}.{format}`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageRequest {
    pub region: Region,
    pub size: Size,
    pub rotation: Rotation,
    pub quality: Quality,
    pub format: Format,
}

/// Which request component failed to parse. Every variant maps to a 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Region,
    Size,
    Rotation,
    Quality,
    Format,
    /// The path didn't have the `region/size/rotation/quality.format` shape.
    Structure,
}

/// A parse failure: the offending component and the input that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub component: Component,
    pub input: String,
}

impl ParseError {
    fn new(component: Component, input: &str) -> Self {
        Self {
            component,
            input: input.to_owned(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.component {
            Component::Region => "region",
            Component::Size => "size",
            Component::Rotation => "rotation",
            Component::Quality => "quality",
            Component::Format => "format",
            Component::Structure => "request path",
        };
        write!(f, "malformed {what}: {:?}", self.input)
    }
}

impl std::error::Error for ParseError {}

/// Strict unsigned decimal integer: one or more ASCII digits, nothing else.
/// Leading zeros are accepted (the canonical print normalizes them away);
/// values that overflow `u32` are rejected — every legal pixel value fits.
fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Strict non-negative decimal float per the spec's float rules: decimal
/// digits and at most one `.`, with digits on both sides (`0.9`, not `.9`
/// or `9.` or `+0.9` or `9e1`).
fn parse_f64(s: &str) -> Option<f64> {
    let valid = match s.split_once('.') {
        Some((int, frac)) => {
            !int.is_empty()
                && !frac.is_empty()
                && int.bytes().all(|b| b.is_ascii_digit())
                && frac.bytes().all(|b| b.is_ascii_digit())
        }
        None => !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
    };
    if !valid {
        return None;
    }
    // Grammar-valid decimal strings always parse; enormous ones saturate to
    // a finite f64 (no exponent syntax exists to produce inf/NaN spellings).
    let v: f64 = s.parse().ok()?;
    v.is_finite().then_some(v)
}

/// Print a float per the spec's canonical float rules: integer spelling if
/// the value is integral, otherwise shortest round-trip decimal.
///
/// Rust's `Display` for `f64` already implements exactly these rules:
/// shortest round-trip decimal, no exponent notation, integral values
/// without a trailing `.0`, sub-one values with the leading `0`, and never
/// a trailing zero.
pub(crate) fn fmt_f64(v: f64) -> String {
    format!("{v}")
}

impl Region {
    /// # Errors
    ///
    /// Returns a [`ParseError`] (HTTP 400) when the input is not a
    /// spec-legal region.
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let err = || ParseError::new(Component::Region, input);
        match input {
            "full" => return Ok(Self::Full),
            "square" => return Ok(Self::Square),
            _ => {}
        }
        if let Some(rest) = input.strip_prefix("pct:") {
            let mut fields = rest.split(',');
            let (x, y, w, h) = (
                fields.next().and_then(parse_f64).ok_or_else(err)?,
                fields.next().and_then(parse_f64).ok_or_else(err)?,
                fields.next().and_then(parse_f64).ok_or_else(err)?,
                fields.next().and_then(parse_f64).ok_or_else(err)?,
            );
            if fields.next().is_some() || w <= 0.0 || h <= 0.0 || x >= 100.0 || y >= 100.0 {
                return Err(err());
            }
            return Ok(Self::Percent { x, y, w, h });
        }
        let mut fields = input.split(',');
        let (x, y, w, h) = (
            fields.next().and_then(parse_u32).ok_or_else(err)?,
            fields.next().and_then(parse_u32).ok_or_else(err)?,
            fields.next().and_then(parse_u32).ok_or_else(err)?,
            fields.next().and_then(parse_u32).ok_or_else(err)?,
        );
        if fields.next().is_some() || w == 0 || h == 0 {
            return Err(err());
        }
        Ok(Self::Pixels { x, y, w, h })
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Full => f.write_str("full"),
            Self::Square => f.write_str("square"),
            Self::Pixels { x, y, w, h } => write!(f, "{x},{y},{w},{h}"),
            Self::Percent { x, y, w, h } => write!(
                f,
                "pct:{},{},{},{}",
                fmt_f64(x),
                fmt_f64(y),
                fmt_f64(w),
                fmt_f64(h)
            ),
        }
    }
}

impl Size {
    /// # Errors
    ///
    /// Returns a [`ParseError`] (HTTP 400) when the input is not a
    /// spec-legal size, including `pct:` values over 100 without the `^`
    /// upscale flag.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let err = || ParseError::new(Component::Size, s);
        let (upscale, rest) = match s.strip_prefix('^') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let kind = if rest == "max" {
            SizeKind::Max
        } else if let Some(pct) = rest.strip_prefix("pct:") {
            let n = parse_f64(pct).ok_or_else(err)?;
            // Without `^`, pct > 100 would upscale — always a 400.
            if n <= 0.0 || (!upscale && n > 100.0) {
                return Err(err());
            }
            SizeKind::Percent(n)
        } else if let Some(confined) = rest.strip_prefix('!') {
            let (w, h) = confined.split_once(',').ok_or_else(err)?;
            let (w, h) = (parse_u32(w).ok_or_else(err)?, parse_u32(h).ok_or_else(err)?);
            if w == 0 || h == 0 {
                return Err(err());
            }
            SizeKind::Confined(w, h)
        } else {
            let (w, h) = rest.split_once(',').ok_or_else(err)?;
            match (w.is_empty(), h.is_empty()) {
                (true, true) => return Err(err()),
                (false, true) => {
                    let w = parse_u32(w).ok_or_else(err)?;
                    if w == 0 {
                        return Err(err());
                    }
                    SizeKind::Width(w)
                }
                (true, false) => {
                    let h = parse_u32(h).ok_or_else(err)?;
                    if h == 0 {
                        return Err(err());
                    }
                    SizeKind::Height(h)
                }
                (false, false) => {
                    let (w, h) = (parse_u32(w).ok_or_else(err)?, parse_u32(h).ok_or_else(err)?);
                    if w == 0 || h == 0 {
                        return Err(err());
                    }
                    SizeKind::WidthHeight(w, h)
                }
            }
        };
        Ok(Self { upscale, kind })
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.upscale {
            f.write_str("^")?;
        }
        match self.kind {
            SizeKind::Max => f.write_str("max"),
            SizeKind::Width(w) => write!(f, "{w},"),
            SizeKind::Height(h) => write!(f, ",{h}"),
            SizeKind::Percent(n) => write!(f, "pct:{}", fmt_f64(n)),
            SizeKind::WidthHeight(w, h) => write!(f, "{w},{h}"),
            SizeKind::Confined(w, h) => write!(f, "!{w},{h}"),
        }
    }
}

impl Rotation {
    /// # Errors
    ///
    /// Returns a [`ParseError`] (HTTP 400) when the input is not a
    /// spec-legal rotation (optional `!`, then degrees in `0..=360`).
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let err = || ParseError::new(Component::Rotation, s);
        let (mirror, rest) = match s.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let degrees = parse_f64(rest).ok_or_else(err)?;
        // "any floating point number from 0 to 360" — inclusive.
        if degrees > 360.0 {
            return Err(err());
        }
        Ok(Self { mirror, degrees })
    }

    /// Whether this rotation is a quarter-turn (0/90/180/270/360), the set
    /// core milestones implement natively; anything else is arbitrary
    /// rotation.
    #[must_use]
    pub fn is_quarter_turn(&self) -> bool {
        self.degrees % 90.0 == 0.0
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mirror {
            f.write_str("!")?;
        }
        f.write_str(&fmt_f64(self.degrees))
    }
}

impl Quality {
    /// # Errors
    ///
    /// Returns a [`ParseError`] (HTTP 400) when the input is not one of
    /// the four v3 quality names.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        match s {
            "default" => Ok(Self::Default),
            "color" => Ok(Self::Color),
            "gray" => Ok(Self::Gray),
            "bitonal" => Ok(Self::Bitonal),
            _ => Err(ParseError::new(Component::Quality, s)),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Color => "color",
            Self::Gray => "gray",
            Self::Bitonal => "bitonal",
        }
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Format {
    /// # Errors
    ///
    /// Returns a [`ParseError`] (HTTP 400) when the input is not one of
    /// the seven spec-enumerated format names.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        match s {
            "jpg" => Ok(Self::Jpg),
            "tif" => Ok(Self::Tif),
            "png" => Ok(Self::Png),
            "gif" => Ok(Self::Gif),
            "jp2" => Ok(Self::Jp2),
            "pdf" => Ok(Self::Pdf),
            "webp" => Ok(Self::Webp),
            _ => Err(ParseError::new(Component::Format, s)),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jpg => "jpg",
            Self::Tif => "tif",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Jp2 => "jp2",
            Self::Pdf => "pdf",
            Self::Webp => "webp",
        }
    }

    #[must_use]
    pub fn media_type(&self) -> &'static str {
        match self {
            Self::Jpg => "image/jpeg",
            Self::Tif => "image/tiff",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Jp2 => "image/jp2",
            Self::Pdf => "application/pdf",
            Self::Webp => "image/webp",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ImageRequest {
    /// Parse the four path segments after the identifier:
    /// `{region}/{size}/{rotation}/{quality}.{format}`.
    ///
    /// The input is the raw path suffix, already split from the identifier
    /// by the router; it must contain exactly three `/` separators.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] (HTTP 400) naming the first component that
    /// failed, or [`Component::Structure`] when the path shape itself is
    /// wrong.
    pub fn parse(path: &str) -> Result<Self, ParseError> {
        let mut segs = path.split('/');
        let (region, size, rotation, last) = (
            segs.next()
                .ok_or_else(|| ParseError::new(Component::Structure, path))?,
            segs.next()
                .ok_or_else(|| ParseError::new(Component::Structure, path))?,
            segs.next()
                .ok_or_else(|| ParseError::new(Component::Structure, path))?,
            segs.next()
                .ok_or_else(|| ParseError::new(Component::Structure, path))?,
        );
        if segs.next().is_some() {
            return Err(ParseError::new(Component::Structure, path));
        }
        // rsplit: quality values never contain '.', so the first '.' from
        // the right separates quality from format.
        let (quality, format) = last
            .rsplit_once('.')
            .ok_or_else(|| ParseError::new(Component::Structure, path))?;
        Ok(Self {
            region: Region::parse(region)?,
            size: Size::parse(size)?,
            rotation: Rotation::parse(rotation)?,
            quality: Quality::parse(quality)?,
            format: Format::parse(format)?,
        })
    }
}

impl fmt::Display for ImageRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}.{}",
            self.region, self.size, self.rotation, self.quality, self.format
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn region(s: &str) -> Region {
        Region::parse(s).unwrap()
    }

    #[test]
    fn region_valid() {
        assert_eq!(region("full"), Region::Full);
        assert_eq!(region("square"), Region::Square);
        assert_eq!(
            region("0,0,1,1"),
            Region::Pixels {
                x: 0,
                y: 0,
                w: 1,
                h: 1
            }
        );
        assert_eq!(
            region("125,15,120,140"),
            Region::Pixels {
                x: 125,
                y: 15,
                w: 120,
                h: 140
            }
        );
        // Leading zeros are accepted; canonical print normalizes.
        assert_eq!(
            region("007,0,1,1"),
            Region::Pixels {
                x: 7,
                y: 0,
                w: 1,
                h: 1
            }
        );
        assert_eq!(
            region("pct:41.6,7.5,40,70"),
            Region::Percent {
                x: 41.6,
                y: 7.5,
                w: 40.0,
                h: 70.0
            }
        );
        assert_eq!(
            region("pct:0,0,100,100"),
            Region::Percent {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0
            }
        );
    }

    #[test]
    fn region_invalid() {
        for s in [
            "",
            "fully",
            "Full",
            "FULL",
            " full",
            "full ",
            "1,2,3",
            "1,2,3,4,5",
            "1,2,3,",
            ",1,2,3",
            "-1,0,1,1",
            "0,0,0,1",
            "0,0,1,0",
            "1.5,0,1,1",
            "0x1,0,1,1",
            "pct:1,2,3",
            "pct:1,2,3,4,5",
            "pct:0,0,0,100",
            "pct:0,0,100,0",
            "pct:100,0,1,1",
            "pct:0,100,1,1",
            "pct:.5,0,1,1",
            "pct:0.,0,1,1",
            "pct:+1,0,1,1",
            "pct:1e2,0,1,1",
            "pct:NaN,0,1,1",
            "pct:inf,0,1,1",
            "4294967296,0,1,1",
            "pct:-0.1,0,1,1",
        ] {
            assert!(Region::parse(s).is_err(), "should reject {s:?}");
        }
    }

    #[track_caller]
    fn size(s: &str) -> Size {
        Size::parse(s).unwrap()
    }

    #[test]
    fn size_valid() {
        assert_eq!(
            size("max"),
            Size {
                upscale: false,
                kind: SizeKind::Max
            }
        );
        assert_eq!(
            size("^max"),
            Size {
                upscale: true,
                kind: SizeKind::Max
            }
        );
        assert_eq!(
            size("150,"),
            Size {
                upscale: false,
                kind: SizeKind::Width(150)
            }
        );
        assert_eq!(
            size("^360,"),
            Size {
                upscale: true,
                kind: SizeKind::Width(360)
            }
        );
        assert_eq!(
            size(",150"),
            Size {
                upscale: false,
                kind: SizeKind::Height(150)
            }
        );
        assert_eq!(
            size("pct:50"),
            Size {
                upscale: false,
                kind: SizeKind::Percent(50.0)
            }
        );
        assert_eq!(
            size("pct:100"),
            Size {
                upscale: false,
                kind: SizeKind::Percent(100.0)
            }
        );
        assert_eq!(
            size("^pct:120"),
            Size {
                upscale: true,
                kind: SizeKind::Percent(120.0)
            }
        );
        assert_eq!(
            size("225,100"),
            Size {
                upscale: false,
                kind: SizeKind::WidthHeight(225, 100)
            }
        );
        assert_eq!(
            size("!225,100"),
            Size {
                upscale: false,
                kind: SizeKind::Confined(225, 100)
            }
        );
        assert_eq!(
            size("^!360,360"),
            Size {
                upscale: true,
                kind: SizeKind::Confined(360, 360)
            }
        );
    }

    #[test]
    fn size_invalid() {
        for s in [
            "",
            "^",
            "full",
            "Max",
            "^^max",
            "pct:0",
            "pct:101",
            "pct:100.001",
            "pct:",
            "^pct:",
            "pct:-1",
            "pct:1e1",
            "0,",
            ",0",
            "0,100",
            "100,0",
            ",",
            "!,",
            "!100,",
            "!,100",
            "!0,1",
            "150",
            "^150",
            "150,,",
            ",,150",
            "1.5,",
            ",1.5",
            "%5Emax",
        ] {
            assert!(Size::parse(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn rotation_valid() {
        assert_eq!(
            Rotation::parse("0").unwrap(),
            Rotation {
                mirror: false,
                degrees: 0.0
            }
        );
        assert_eq!(
            Rotation::parse("90").unwrap(),
            Rotation {
                mirror: false,
                degrees: 90.0
            }
        );
        assert_eq!(
            Rotation::parse("360").unwrap(),
            Rotation {
                mirror: false,
                degrees: 360.0
            }
        );
        assert_eq!(
            Rotation::parse("22.5").unwrap(),
            Rotation {
                mirror: false,
                degrees: 22.5
            }
        );
        assert_eq!(
            Rotation::parse("!0").unwrap(),
            Rotation {
                mirror: true,
                degrees: 0.0
            }
        );
        assert_eq!(
            Rotation::parse("!337.5").unwrap(),
            Rotation {
                mirror: true,
                degrees: 337.5
            }
        );
    }

    #[test]
    fn rotation_invalid() {
        for s in [
            "", "!", "-0", "-90", "360.001", "361", "1e2", ".5", "90.", "+90", "9O",
        ] {
            assert!(Rotation::parse(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn quarter_turns() {
        for (s, expect) in [
            ("0", true),
            ("90", true),
            ("180", true),
            ("270", true),
            ("360", true),
            ("22.5", false),
            ("90.1", false),
        ] {
            assert_eq!(Rotation::parse(s).unwrap().is_quarter_turn(), expect, "{s}");
        }
    }

    #[test]
    fn quality_and_format() {
        assert_eq!(Quality::parse("default").unwrap(), Quality::Default);
        assert_eq!(Quality::parse("bitonal").unwrap(), Quality::Bitonal);
        assert!(Quality::parse("native").is_err()); // v2 name, not v3
        assert!(Quality::parse("grey").is_err());
        assert_eq!(Format::parse("jpg").unwrap(), Format::Jpg);
        assert_eq!(Format::parse("webp").unwrap(), Format::Webp);
        assert!(Format::parse("jpeg").is_err());
        assert!(Format::parse("tiff").is_err());
        assert!(Format::parse("JPG").is_err());
    }

    #[test]
    fn full_request() {
        let r = ImageRequest::parse("full/max/0/default.jpg").unwrap();
        assert_eq!(r.to_string(), "full/max/0/default.jpg");
        let r = ImageRequest::parse("125,15,120,140/90,/!345.3/gray.png").unwrap();
        assert_eq!(r.to_string(), "125,15,120,140/90,/!345.3/gray.png");
        for s in [
            "full/max/0/default",
            "full/max/0.jpg",
            "full/max/0/default.jpg/extra",
            "/full/max/0/default.jpg",
            "full/max/0/default.jpg.png",
        ] {
            assert!(ImageRequest::parse(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn float_canonical_print() {
        assert_eq!(fmt_f64(0.0), "0");
        assert_eq!(fmt_f64(90.0), "90");
        assert_eq!(fmt_f64(22.5), "22.5");
        assert_eq!(fmt_f64(0.9), "0.9");
        assert_eq!(fmt_f64(360.0), "360");
    }
}
