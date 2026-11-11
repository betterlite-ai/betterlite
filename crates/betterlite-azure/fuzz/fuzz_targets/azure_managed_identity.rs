#![no_main]

use betterlite_azure::azure_managed_identity::decode::decode_imds_token;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_imds_token(bytes);
});
