#![no_main]

use betterlite_server::guardrails::custom_code_test::body::decode_test_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(decode_test_request(body));
});
