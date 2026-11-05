#![no_main]

use libfuzzer_sys::fuzz_target;
use std::str;

#[path = "../../src/case/browser_flow/form.rs"]
pub mod form;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = str::from_utf8(bytes)
        .map_err(form::FormCaptureError::Utf8)
        .and_then(form::parse_form_projection);
});
