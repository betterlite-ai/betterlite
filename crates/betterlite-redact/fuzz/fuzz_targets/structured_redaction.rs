#![no_main]

use betterlite_redact::structured::{
    mask_credentials_in_payload, mask_sensitive_structure, redact::redact_credentials_in_payload,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
        let _ = mask_sensitive_structure(&value);
        let _ = mask_credentials_in_payload(&value);
        let _ = redact_credentials_in_payload(&value);
    }
});
