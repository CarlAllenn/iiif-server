//! Every decoder boundary: hostile bytes may be rejected, never panic.
//! Successful opens additionally serve one bounded crop through the full
//! pipeline (decode → resize → encode).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let cursor = std::io::Cursor::new(data.to_vec());
    let Ok(mut master) = iiif_core::codec::open_master(cursor) else {
        return;
    };
    let (w, h) = master.dimensions();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16_000_000 {
        return; // decompression-bomb guard mirrors the server's limits
    }
    let _ = master.describe();
    let limits = iiif_core::info::Limits {
        max_width: 512,
        max_height: 512,
        max_area: 262_144,
    };
    let Ok(request) = iiif_core::grammar::ImageRequest::parse("full/!64,64/90/gray.png") else {
        return;
    };
    if let Ok(plan) = iiif_core::eval::evaluate(&request, w, h, limits) {
        let _ = iiif_core::pipeline::execute(master.as_mut(), &plan);
    }
});
