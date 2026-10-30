#![no_main]

use betterlite_openai::embedding::{EmbeddingRequest, EmbeddingResponse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _request = EmbeddingRequest::parse(bytes);
    let _response = serde_json::from_slice::<EmbeddingResponse>(bytes);
});
