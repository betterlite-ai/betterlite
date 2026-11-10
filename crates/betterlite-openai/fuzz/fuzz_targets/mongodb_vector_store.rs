#![no_main]

use betterlite_openai::mongodb_vector_store::decode::decode_mongodb_search;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_mongodb_search(bytes);
});
