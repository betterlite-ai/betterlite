#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = betterlite::guardrails::pattern_matching::blocked_words::decode(bytes);
});
