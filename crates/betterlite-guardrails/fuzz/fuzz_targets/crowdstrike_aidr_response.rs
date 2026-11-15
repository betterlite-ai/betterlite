#![no_main]

use betterlite_guardrails::crowdstrike_aidr::models::{GuardResponse, GuardSignals};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _signals = serde_json::from_slice::<GuardSignals>(bytes);
    let _response = serde_json::from_slice::<GuardResponse>(bytes);
});
