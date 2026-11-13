#![no_main]

use betterlite_vertex::cache_create::records::decode_created_cache;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_created_cache(bytes);
});
