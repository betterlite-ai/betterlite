#![no_main]

use betterlite_openai::dashscope_embedding::reply::decode_dashscope_embedding;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_dashscope_embedding(bytes, 200, "text-embedding-v4");
});
