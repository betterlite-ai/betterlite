#![no_main]

use betterlite_azure::responses::decode::decode_azure_response_resource;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = decode_azure_response_resource(bytes);
});
