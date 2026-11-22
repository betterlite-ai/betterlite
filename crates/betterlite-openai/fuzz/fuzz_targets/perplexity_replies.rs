#![no_main]

use betterlite_openai::perplexity_embedding::reply::transform_embedding_response;
use betterlite_openai::perplexity_search::reply::decode_search_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _embedding = transform_embedding_response("pplx-embed-v1-0.6b", 200, bytes);
    let _search = decode_search_reply(bytes);
});
