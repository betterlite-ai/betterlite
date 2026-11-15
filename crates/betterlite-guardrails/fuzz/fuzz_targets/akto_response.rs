#![no_main]

use betterlite_guardrails::akto::transport::parse_verdict;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _verdict = parse_verdict(bytes);
});
