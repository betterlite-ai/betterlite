#![no_main]

use betterlite_router::retry_policy::retry_after::parse_retry_after;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = parse_retry_after(data);
});
