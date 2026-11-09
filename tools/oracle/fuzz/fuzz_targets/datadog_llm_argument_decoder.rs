#![no_main]
use std::str::from_utf8;

use betterlite_logging::datadog_llm_obs::arguments::decode_tool_arguments;
use betterlite_logging::datadog_llm_obs::literal::literal_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = from_utf8(bytes) {
        let _decoded = decode_tool_arguments(text);
        let _literal = literal_json(text);
    }
});
