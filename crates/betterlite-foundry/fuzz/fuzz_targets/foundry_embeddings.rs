#![no_main]

use betterlite_foundry::embeddings::reply::decode_foundry_embedding;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_foundry_embedding(bytes);
});
