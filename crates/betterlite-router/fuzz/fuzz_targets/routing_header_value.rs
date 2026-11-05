#![no_main]

use betterlite_router::routing_decision_headers::decode_routing_header_value;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|value: &str| {
    let _outcome = decode_routing_header_value(value);
});
