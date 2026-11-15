#![no_main]

use betterlite_openai::huggingface_rerank::response::decode_rerank_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_rerank_response(bytes, Some(200));
});
