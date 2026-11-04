#![no_main]

use betterlite_models::json_provider_registry::{DecodeLimits, decode_json_provider_registry};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = decode_json_provider_registry(input, DecodeLimits::default());
});
