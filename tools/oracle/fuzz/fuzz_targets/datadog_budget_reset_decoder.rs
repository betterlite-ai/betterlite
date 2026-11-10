#![no_main]
use std::str::from_utf8;

use betterlite_logging::datadog_llm_privacy::datadog_budget_reset_decoder::decode_budget_reset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = from_utf8(bytes) {
        let _decoded = decode_budget_reset(text);
    }
});
