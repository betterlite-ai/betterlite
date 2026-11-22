#![no_main]

use betterlite_server::guardrails::submissions::models::decode_register_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(decode_register_body(body));
});
