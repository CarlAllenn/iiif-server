//! The v2.1 translation grammar is equally total.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let _ = iiif_core::v2::parse_image_request(input);
});
