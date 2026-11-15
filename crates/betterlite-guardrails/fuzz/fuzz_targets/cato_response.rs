#![no_main]

use betterlite_guardrails::cato::wire::AnalyzeResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = serde_json::from_slice::<AnalyzeResponse>(bytes);
});
