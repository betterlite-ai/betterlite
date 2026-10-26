#![no_main]
use betterlite_harness::payloads::{decode_json_line, last_json_object};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|line: &[u8]| {
    let _ = decode_json_line(line);
    if let Ok(text) = std::str::from_utf8(line) {
        let _ = last_json_object(text);
    }
});
