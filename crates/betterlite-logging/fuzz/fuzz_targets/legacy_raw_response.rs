#![no_main]

use betterlite_logging::otel::legacy_attributes::raw_request::parse_raw_response;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        drop(parse_raw_response(text));
    }
});
