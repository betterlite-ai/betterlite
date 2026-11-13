#![no_main]

use betterlite_prompts::gemini_tool_invokes::invoke::convert_tool_calls_to_gemini;
use betterlite_prompts::gemini_tool_invokes::parser::parse_tool_invoke_message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(message) = parse_tool_invoke_message(bytes) else {
        return;
    };
    drop(convert_tool_calls_to_gemini(
        &message,
        Some("gemini-3-pro-preview"),
        true,
    ));
    drop(convert_tool_calls_to_gemini(&message, None, false));
});
