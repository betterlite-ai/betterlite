#![no_main]

use betterlite_guardrails::qualifire::wire::Verdict;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _verdict = serde_json::from_slice::<Verdict>(bytes);
});
