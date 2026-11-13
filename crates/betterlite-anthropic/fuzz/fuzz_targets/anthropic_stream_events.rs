#![no_main]

use std::str;

use betterlite_anthropic::stream_events::decode_stream_tool_arguments;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(arguments) = str::from_utf8(data) {
        let _outcome = decode_stream_tool_arguments(arguments);
    }
});
