#![no_main]

use betterlite_context::local_ttl_cache::decode_cached_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _decoded = decode_cached_json(input);
});
