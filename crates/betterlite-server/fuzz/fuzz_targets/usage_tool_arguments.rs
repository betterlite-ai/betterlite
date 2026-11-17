#![no_main]

use betterlite_server::usage_ai_chat::fetch_args::decode_tool_arguments;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _decoded = decode_tool_arguments(text);
});
