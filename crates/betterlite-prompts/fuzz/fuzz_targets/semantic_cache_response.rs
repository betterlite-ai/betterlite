#![no_main]

use betterlite_prompts::semantic_cache_records::decode_cached_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_cached_response(Some(bytes)));
});
