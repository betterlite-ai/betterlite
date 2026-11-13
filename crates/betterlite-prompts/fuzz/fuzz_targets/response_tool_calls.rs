#![no_main]

use betterlite_prompts::response_tool_calls::normalized::get_tool_calls_from_response;
use betterlite_prompts::response_tool_calls::projections::parse_tool_call_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
        return;
    }
    if let Ok(response) = parse_tool_call_response(bytes) {
        drop(get_tool_calls_from_response(&response, false));
        drop(get_tool_calls_from_response(&response, true));
    }
});
