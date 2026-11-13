#![no_main]

use std::str::from_utf8;

use betterlite_vertex::gemini_history::content::decode_thinking_part;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(thinking) = from_utf8(bytes) {
        drop(decode_thinking_part(thinking, "signature"));
    }
});
