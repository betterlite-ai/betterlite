#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::api::vector_management_boundaries::json::decode_management_json;
use betterlite_server::api::vector_management_boundaries::redaction::redact_litellm_params;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = ApiLimits::default();
    if let Ok(value) = decode_management_json(bytes, limits) {
        drop(redact_litellm_params(&value, 0, limits));
    }
});
