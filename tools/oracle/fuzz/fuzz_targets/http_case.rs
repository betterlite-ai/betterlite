#![no_main]

use betterlite_oracle::case::decode_http_case;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_http_case(bytes);
});
