#![no_main]

use betterlite_anthropic::credentials::models::decode_anthropic_models;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _success = decode_anthropic_models(200, input);
    let _failure = decode_anthropic_models(500, input);
});
