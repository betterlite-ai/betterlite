#![no_main]

use betterlite_oracle::sdk_http::{OracleLimits, decode_sdk_http_case, decode_sdk_http_result};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = OracleLimits::default();
    match bytes.split_first() {
        Some((0, case)) => drop(decode_sdk_http_case(case, &limits)),
        Some((1, result)) => drop(decode_sdk_http_result(result, &limits)),
        Some(_) | None => {}
    }
});
