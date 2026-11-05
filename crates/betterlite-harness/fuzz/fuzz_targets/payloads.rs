#![no_main]
use betterlite_harness::payloads::{decode_json_line, last_json_object};
use libfuzzer_sys::fuzz_target;
use std::str;
fuzz_target!(|line: &[u8]| {
    let _decoded_line = decode_json_line(line);
    if let Ok(text) = str::from_utf8(line) {
        let _last_object = last_json_object(text);
    }
});
