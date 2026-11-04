#![no_main]

use betterlite_core::routes::{get_call_types_for_route, parse_route};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(route) = std::str::from_utf8(bytes) {
        drop(parse_route(route));
        drop(get_call_types_for_route(route));
    }
});
