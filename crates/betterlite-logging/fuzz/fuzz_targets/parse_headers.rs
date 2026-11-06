#![no_main]

use betterlite_logging::otel::plumbing::headers::parse_headers;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        let _headers = parse_headers(Some(text));
    }
});
