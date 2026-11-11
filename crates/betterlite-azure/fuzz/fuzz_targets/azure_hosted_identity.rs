#![no_main]

use betterlite_azure::azure_hosted_identity::decode::decode_hosted_identity_token;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_hosted_identity_token(bytes);
});
