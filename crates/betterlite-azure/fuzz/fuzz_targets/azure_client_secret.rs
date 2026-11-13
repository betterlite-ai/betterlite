#![no_main]

use betterlite_azure::azure_client_secret::token::decode_client_secret_token;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _token = decode_client_secret_token(bytes);
});
