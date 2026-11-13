#![no_main]

use betterlite_tokenizer::tools::{
    ToolLimits, format_function_definitions, parse_tool_definitions,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(tools) = parse_tool_definitions(bytes, ToolLimits::default()) {
        let _formatted = format_function_definitions(&tools);
    }
});
