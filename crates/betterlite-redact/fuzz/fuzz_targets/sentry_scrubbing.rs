#![no_main]

use betterlite_redact::sentry::pattern::scrub;
use libfuzzer_sys::fuzz_target;
use std::str::from_utf8;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let _unscrubbed_mode = scrub(text, false);
    let _scrubbed_mode = scrub(text, true);
});
