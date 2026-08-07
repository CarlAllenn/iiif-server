//! Identifier decoding is a named security component: never panics, and
//! nothing that decodes successfully contains a traversal shape.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(id) = iiif_core::ident::Identifier::decode(input) {
        let path = id.as_path();
        assert!(!path.starts_with('/'));
        assert!(!path.contains('\\'));
        for segment in path.split('/') {
            assert!(!segment.is_empty());
            assert_ne!(segment, ".");
            assert_ne!(segment, "..");
        }
        // Re-encoding must round-trip.
        let re = iiif_core::ident::Identifier::decode(&id.encoded());
        assert_eq!(re, Ok(id));
    }
});
