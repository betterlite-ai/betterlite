#![no_main]

use betterlite_openai::watsonx_auth::iam::decode_iam_token_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_iam_token_response(bytes);
});
