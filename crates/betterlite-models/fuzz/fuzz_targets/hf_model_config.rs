#![no_main]

use betterlite_models::model_metadata_resolution::hf_config::decode_hf_model_config;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _outcome = decode_hf_model_config(input);
});
