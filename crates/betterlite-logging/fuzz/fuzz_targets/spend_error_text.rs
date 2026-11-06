#![no_main]

use betterlite_logging::spend_sanitize::prompt_leaks::redact_spend_error_text;
use libfuzzer_sys::fuzz_target;
use std::str;

const MAX_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    let Some(bytes) = data.get(..data.len().min(MAX_BYTES)) else {
        return;
    };
    if let Ok(text) = str::from_utf8(bytes) {
        drop(redact_spend_error_text(text));
    }
});
