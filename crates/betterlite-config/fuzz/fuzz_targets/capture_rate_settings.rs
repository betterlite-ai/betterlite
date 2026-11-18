#![no_main]

use std::str::from_utf8;

use betterlite_config::capture_rate::decode_capture_rate_settings;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let _decoded = decode_capture_rate_settings(text);
});
