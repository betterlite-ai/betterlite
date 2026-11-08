#![no_main]

use betterlite_guardrails::pattern_matching::blocked_words::decode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = decode(bytes);
});
