#![no_main]

use betterlite_vertex::google_count_tokens::decode::decode_google_count;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_google_count(bytes);
});
