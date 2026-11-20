#![no_main]

use betterlite_guardrails::bedrock_records::ChecksResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = serde_json::from_slice::<ChecksResponse>(bytes);
});
