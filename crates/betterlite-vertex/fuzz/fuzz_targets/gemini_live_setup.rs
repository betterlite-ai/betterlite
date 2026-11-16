#![no_main]

use std::str::from_utf8;

use betterlite_vertex::gemini_live_setup::decode::decode_gemini_live_setup;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(request) = from_utf8(bytes) else {
        return;
    };
    drop(decode_gemini_live_setup(request));
});
