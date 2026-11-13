#![no_main]

use betterlite_router::routing_value_store::codec::parse_route_value;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = parse_route_value(data);
});
