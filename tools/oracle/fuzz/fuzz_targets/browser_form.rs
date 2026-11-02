#![no_main]

use betterlite_oracle::case::decode_html_form_capture;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_html_form_capture(bytes);
});
