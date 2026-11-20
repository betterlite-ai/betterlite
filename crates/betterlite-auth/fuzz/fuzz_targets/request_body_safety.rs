#![no_main]

use betterlite_auth::request_body_safety::decode_request_metadata;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_request_metadata(bytes));
});
