#![no_main]

use betterlite_cache::types::record::decode_cache_types;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_cache_types(bytes);
});
