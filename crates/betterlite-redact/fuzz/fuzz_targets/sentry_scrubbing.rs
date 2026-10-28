#![no_main]

use betterlite_redact::sentry::pattern::scrub;
use libfuzzer_sys::fuzz_target;
use std::str::from_utf8;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let _ = scrub(text, false);
    let _ = scrub(text, true);
});
