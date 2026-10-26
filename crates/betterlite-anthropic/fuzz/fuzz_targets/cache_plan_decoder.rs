#![no_main]

use std::str;

use betterlite_anthropic::cache_plan::{parse_cache_plan, parse_prompt};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(body) = str::from_utf8(bytes) {
        drop(parse_cache_plan(body));
        drop(parse_prompt(body));
    }
});
