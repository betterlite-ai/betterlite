#![no_main]

use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::custom_routes::decode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = decode(bytes);
});
