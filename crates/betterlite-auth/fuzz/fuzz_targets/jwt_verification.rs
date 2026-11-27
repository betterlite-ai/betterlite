#![no_main]

use betterlite_auth::jwt_verification::decode::decode_signed_jwt;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_signed_jwt(bytes));
});
