#![no_main]
use betterlite_server::guardrails::catalog_apply::blocked_words::validate_blocked_words;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(validate_blocked_words(bytes));
});
