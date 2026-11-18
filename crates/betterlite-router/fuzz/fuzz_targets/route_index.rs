#![no_main]

use std::str;

use betterlite_router::semantic_router::index::parse_inline;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(config) = str::from_utf8(bytes) else {
        return;
    };
    let _outcome = parse_inline(config);
});
