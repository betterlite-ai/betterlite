#![no_main]

use betterlite_openai::xai_identity::decode_xai_models;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _success = decode_xai_models(200, bytes);
    let _failure = decode_xai_models(500, bytes);
});
