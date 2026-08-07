//! The v3 grammar must be total: no input panics it, and every accepted
//! value round-trips through its canonical printing.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(request) = iiif_core::grammar::ImageRequest::parse(input) {
        let printed = request.to_string();
        let reparsed = iiif_core::grammar::ImageRequest::parse(&printed);
        assert_eq!(reparsed, Ok(request), "round-trip mismatch for {input:?}");
    }
});
