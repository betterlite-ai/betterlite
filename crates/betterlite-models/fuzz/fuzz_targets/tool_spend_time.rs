#![no_main]

use std::str;

use betterlite_models::tool_spend::decode_tool_spend_time;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let _decoded = decode_tool_spend_time(text);
});
