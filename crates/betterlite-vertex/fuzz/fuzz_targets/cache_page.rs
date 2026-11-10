#![no_main]

use betterlite_vertex::cache_lookup::records::decode_cached_page;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_cached_page(bytes);
});
