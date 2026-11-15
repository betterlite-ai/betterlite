#![no_main]

use betterlite_guardrails::compresr_transport::response::parse_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _single = parse_body(bytes, 1);
    let _batch = parse_body(bytes, 2);
});
