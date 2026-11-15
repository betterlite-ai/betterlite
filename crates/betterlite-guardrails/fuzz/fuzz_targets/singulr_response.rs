#![no_main]

use betterlite_guardrails::singulr::wire::Answer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _answer = serde_json::from_slice::<Answer>(bytes);
});
