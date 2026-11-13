#![no_main]

use betterlite_router::capability_forecast::reply::parse_cap_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = parse_cap_reply(data);
});
