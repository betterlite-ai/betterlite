#![no_main]

use betterlite_guardrails::aim::stream::StreamFrame;
use betterlite_guardrails::aim::wire::AnalyzeResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = serde_json::from_slice::<AnalyzeResponse>(bytes);
    let _frame = serde_json::from_slice::<StreamFrame>(bytes);
});
