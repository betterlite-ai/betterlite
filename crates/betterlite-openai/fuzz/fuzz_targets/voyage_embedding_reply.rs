#![no_main]

use betterlite_openai::voyage_embedding::{VoyageEmbeddingVariant, decode_voyage_embedding_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _text = decode_voyage_embedding_reply(bytes, 502, VoyageEmbeddingVariant::Text);
    let _contextual = decode_voyage_embedding_reply(bytes, 502, VoyageEmbeddingVariant::Contextual);
});
