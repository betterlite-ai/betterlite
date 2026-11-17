#![no_main]

use betterlite_guardrails::agent_365::enforcement_types::decode_evaluate_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _verdict = decode_evaluate_response(bytes);
});
