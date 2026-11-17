#![no_main]

use betterlite_server::key_fields::decode::{decode_key_request, decode_key_response};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_key_request(bytes));
    drop(decode_key_response(bytes));
});
