#![no_main]

use betterlite_core::skills::sandbox::decoder::{MAX_LISTING_ENTRIES, decode_generated_paths};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let decoded = decode_generated_paths(text);
    assert!(decoded.paths.len() <= MAX_LISTING_ENTRIES);
});
