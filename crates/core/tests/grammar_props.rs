//! Property tests: parse ↔ print round-trips for the whole grammar, plus
//! parser total-safety (never panics on arbitrary input).

use iiif_core::grammar::{Format, ImageRequest, Quality, Region, Rotation, Size, SizeKind};
use proptest::prelude::*;

/// Grammar-representable floats: built from decimal strings so every
/// generated value has an exact spelling the parser accepts.
fn decimal_f64(int_max: u32) -> impl Strategy<Value = f64> {
    (0..=int_max, proptest::option::of("[0-9]{1,6}")).prop_map(|(int, frac)| {
        let s = match frac {
            Some(frac) => format!("{int}.{frac}"),
            None => format!("{int}"),
        };
        s.parse().unwrap()
    })
}

fn positive_decimal_f64(int_max: u32) -> impl Strategy<Value = f64> {
    decimal_f64(int_max).prop_filter("must be positive", |v| *v > 0.0)
}

fn region_strategy() -> impl Strategy<Value = Region> {
    prop_oneof![
        Just(Region::Full),
        Just(Region::Square),
        (any::<u32>(), any::<u32>(), 1..=u32::MAX, 1..=u32::MAX)
            .prop_map(|(x, y, w, h)| Region::Pixels { x, y, w, h }),
        (
            decimal_f64(99),
            decimal_f64(99),
            positive_decimal_f64(100),
            positive_decimal_f64(100),
        )
            .prop_map(|(x, y, w, h)| Region::Percent { x, y, w, h }),
    ]
}

fn size_strategy() -> impl Strategy<Value = Size> {
    let kind = prop_oneof![
        Just(SizeKind::Max),
        (1..=u32::MAX).prop_map(SizeKind::Width),
        (1..=u32::MAX).prop_map(SizeKind::Height),
        positive_decimal_f64(200).prop_map(SizeKind::Percent),
        (1..=u32::MAX, 1..=u32::MAX).prop_map(|(w, h)| SizeKind::WidthHeight(w, h)),
        (1..=u32::MAX, 1..=u32::MAX).prop_map(|(w, h)| SizeKind::Confined(w, h)),
    ];
    kind.prop_flat_map(|kind| {
        // pct > 100 is only legal with the `^` flag.
        let upscale = match kind {
            SizeKind::Percent(n) if n > 100.0 => Just(true).boxed(),
            _ => any::<bool>().boxed(),
        };
        upscale.prop_map(move |upscale| Size { upscale, kind })
    })
}

fn rotation_strategy() -> impl Strategy<Value = Rotation> {
    (
        any::<bool>(),
        decimal_f64(360).prop_filter("0..=360", |v| *v <= 360.0),
    )
        .prop_map(|(mirror, degrees)| Rotation { mirror, degrees })
}

fn quality_strategy() -> impl Strategy<Value = Quality> {
    prop_oneof![
        Just(Quality::Default),
        Just(Quality::Color),
        Just(Quality::Gray),
        Just(Quality::Bitonal),
    ]
}

fn format_strategy() -> impl Strategy<Value = Format> {
    prop_oneof![
        Just(Format::Jpg),
        Just(Format::Tif),
        Just(Format::Png),
        Just(Format::Gif),
        Just(Format::Jp2),
        Just(Format::Pdf),
        Just(Format::Webp),
    ]
}

fn request_strategy() -> impl Strategy<Value = ImageRequest> {
    (
        region_strategy(),
        size_strategy(),
        rotation_strategy(),
        quality_strategy(),
        format_strategy(),
    )
        .prop_map(|(region, size, rotation, quality, format)| ImageRequest {
            region,
            size,
            rotation,
            quality,
            format,
        })
}

proptest! {
    /// Print then parse is the identity on every valid typed value.
    #[test]
    fn region_roundtrip(r in region_strategy()) {
        prop_assert_eq!(Region::parse(&r.to_string()).unwrap(), r);
    }

    #[test]
    fn size_roundtrip(s in size_strategy()) {
        prop_assert_eq!(Size::parse(&s.to_string()).unwrap(), s);
    }

    #[test]
    fn rotation_roundtrip(r in rotation_strategy()) {
        prop_assert_eq!(Rotation::parse(&r.to_string()).unwrap(), r);
    }

    #[test]
    fn request_roundtrip(r in request_strategy()) {
        prop_assert_eq!(ImageRequest::parse(&r.to_string()).unwrap(), r);
    }

    /// Printing is idempotent: parse(print(v)) prints identically —
    /// i.e. printed forms are already canonical spellings.
    #[test]
    fn print_is_canonical(r in request_strategy()) {
        let printed = r.to_string();
        let reparsed = ImageRequest::parse(&printed).unwrap();
        prop_assert_eq!(reparsed.to_string(), printed);
    }

    /// The parser is total: arbitrary bytes never panic it.
    #[test]
    fn parser_never_panics(s in "\\PC*") {
        let _ = ImageRequest::parse(&s);
        let _ = Region::parse(&s);
        let _ = Size::parse(&s);
        let _ = Rotation::parse(&s);
        let _ = Quality::parse(&s);
        let _ = Format::parse(&s);
    }

    /// Parsing accepts leading zeros but prints them away (canonical), and
    /// reparse of the canonical form equals the original parse.
    #[test]
    fn leading_zero_normalization(x in 0u32..1000, y in 0u32..1000, w in 1u32..1000, h in 1u32..1000) {
        let padded = format!("{x:07},{y:07},{w:07},{h:07}");
        let parsed = Region::parse(&padded).unwrap();
        let canonical = parsed.to_string();
        prop_assert_eq!(Region::parse(&canonical).unwrap(), parsed);
        prop_assert!(!canonical.contains(",0") || canonical.split(',').all(|p| p == "0" || !p.starts_with('0')));
    }
}
