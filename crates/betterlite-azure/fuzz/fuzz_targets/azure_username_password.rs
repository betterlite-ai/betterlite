#![no_main]

use betterlite_azure::azure_username_password::token::decode_password_token;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _token = decode_password_token(bytes);
});
