#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = betterlite_harness::payloads::decode_json_line(input);
    if let Ok(text) = std::str::from_utf8(input) {
        let _ = betterlite_harness::payloads::last_json_object(text);
    }
});
