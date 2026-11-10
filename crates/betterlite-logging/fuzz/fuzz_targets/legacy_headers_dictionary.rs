#![no_main]

use betterlite_logging::otel::legacy_exporters::headers::{HeadersInput, headers_dictionary};
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        let _headers = headers_dictionary(HeadersInput::Text(text));
    }
});
