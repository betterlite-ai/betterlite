#![no_main]

use betterlite_provider::watsonx_embed::reply::decode_watsonx_embedding_reply;
use betterlite_provider::watsonx_rerank::reply::decode_watsonx_rerank_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _embedding = decode_watsonx_embedding_reply(bytes);
    let _rerank = decode_watsonx_rerank_reply(bytes);
});
