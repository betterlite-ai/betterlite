#![no_main]

use betterlite_server::cost_estimate_route::decode::decode_cost_estimate_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_cost_estimate_request(bytes));
});
