#![no_main]

use betterlite_bedrock::realtime_completion::tool_input::decode_realtime_tool_input;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    drop(decode_realtime_tool_input(input));
});
