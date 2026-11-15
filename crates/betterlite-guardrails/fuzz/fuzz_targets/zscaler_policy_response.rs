#![no_main]

use betterlite_guardrails::zscaler_ai_guard::wire::PolicyResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = serde_json::from_slice::<PolicyResponse>(bytes);
});
