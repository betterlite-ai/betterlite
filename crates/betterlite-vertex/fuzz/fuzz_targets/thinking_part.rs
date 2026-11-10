#![no_main]

use std::str::from_utf8;

use betterlite_vertex::gemini_history::thinking::decode_thinking_part;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(thinking) = from_utf8(bytes) else {
        return;
    };
    drop(decode_thinking_part(thinking, "signature"));
    drop(decode_thinking_part(thinking, ""));
});
