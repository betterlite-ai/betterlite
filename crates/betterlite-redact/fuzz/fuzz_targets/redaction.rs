#![no_main]

use std::str::from_utf8;

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let Ok(redactor) = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH) else {
        return;
    };
    let _redacted_string = redactor.redact_string(text);
    let _redacted_details = redactor.redact_internal_details(text);
    let _redacted_keyed = redactor.redact_structured_value(Some("password"), text);
    let _redacted_unkeyed = redactor.redact_structured_value(None, text);
});
