#![no_main]

use betterlite_redact::sentry::scrub::scrub_text;
use libfuzzer_sys::fuzz_target;
use std::str::from_utf8;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let _ = scrub_text(text, false);
    let _ = scrub_text(text, true);
});
