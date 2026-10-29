#![no_main]

use betterlite_prompts::arguments::concatenated::{
    salvage_concatenated_arguments, split_concatenated_json,
};
use betterlite_prompts::arguments::parse::parse_tool_call_arguments;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    if let Ok(arguments) = str::from_utf8(bytes) {
        drop(parse_tool_call_arguments(arguments, None, None));
        drop(split_concatenated_json(arguments));
        drop(salvage_concatenated_arguments(arguments));
    }
});
