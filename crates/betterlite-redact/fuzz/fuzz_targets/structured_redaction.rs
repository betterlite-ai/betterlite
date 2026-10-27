#![no_main]

use betterlite_redact::structured::{
    mask_credentials_in_payload, mask_sensitive_structure, parse::parse_payload,
    redact::redact_credentials_in_payload,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(value) = parse_payload(bytes) {
        let _ = mask_sensitive_structure(&value);
        let _ = mask_credentials_in_payload(&value);
        if let Some(mapping) = value.as_object() {
            let _ = redact_credentials_in_payload(mapping);
        }
    }
});
