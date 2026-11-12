#![no_main]

use betterlite_openai::cometapi_wire::embed::{EmbeddingReplyInput, decode_comet_embedding_reply};
use betterlite_openai::openai_base_config::Headers;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let headers = Headers::new();
    let _reply = decode_comet_embedding_reply(&EmbeddingReplyInput {
        body: bytes,
        status: 200,
        headers: &headers,
    });
});
