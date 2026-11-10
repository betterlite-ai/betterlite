#![no_main]

use betterlite_azure::oidc_exchange::decode_azure_oidc_token;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _token = decode_azure_oidc_token(bytes);
});
