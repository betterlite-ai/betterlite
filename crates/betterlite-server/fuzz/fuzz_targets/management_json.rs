#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::api::vector_management_boundaries::json::decode_management_json;
use betterlite_server::api::vector_management_boundaries::redaction::LitellmParamsRedactor;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = ApiLimits::default();
    if let Ok(value) = decode_management_json(bytes, limits) {
        drop(LitellmParamsRedactor::new().redact(&value, 0, limits));
    }
});
