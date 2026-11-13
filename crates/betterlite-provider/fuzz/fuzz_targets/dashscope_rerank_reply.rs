#![no_main]

use betterlite_provider::dashscope_rerank::decode_dashscope_rerank_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_dashscope_rerank_reply(bytes, 200);
});
