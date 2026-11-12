#![no_main]

use std::str::from_utf8;

use betterlite_vertex::multimodal_embedding_records::media::classify_embedding_media;
use betterlite_vertex::multimodal_embedding_records::response::decode_multimodal_embedding;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = from_utf8(bytes) {
        let _classified = classify_embedding_media(text);
    }
    let _decoded = decode_multimodal_embedding(bytes);
});
