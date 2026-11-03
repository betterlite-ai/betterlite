#![no_main]

use betterlite_oracle::case::browser_flow::form::{FormCaptureError, parse_form_projection};
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    let _ = str::from_utf8(bytes)
        .map_err(FormCaptureError::Utf8)
        .and_then(parse_form_projection);
});
