#![no_main]

use betterlite_a2a::watsonx_credentials::token_response::{TokenFlow, decode_wxo_token_response};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ibm_cloud = decode_wxo_token_response(bytes, TokenFlow::IbmCloud, 1_700_000_000);
    let _cp4d = decode_wxo_token_response(bytes, TokenFlow::Cp4d, 1_700_000_000);
});
