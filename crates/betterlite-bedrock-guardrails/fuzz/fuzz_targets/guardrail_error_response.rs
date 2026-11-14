#![no_main]

use betterlite_bedrock_guardrails::guardrail_error_response::parse_error_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (u16, &[u8])| {
    let (status_code, body) = input;
    let _outcome = parse_error_response(status_code, body);
});
