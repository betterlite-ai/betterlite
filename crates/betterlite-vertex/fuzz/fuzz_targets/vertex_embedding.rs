#![no_main]

use betterlite_vertex::vertex_embedding_call::decode::decode_vertex_embedding;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_vertex_embedding(bytes));
});
