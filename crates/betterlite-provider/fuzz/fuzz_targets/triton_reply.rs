#![no_main]

use std::str;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_provider::triton::embedding::TextTokenCounter;
use betterlite_provider::triton::embedding::transform_embedding_request;
use betterlite_provider::triton::embedding::transform_embedding_response;
use betterlite_provider::triton::generate::transform_response as generate_response;
use betterlite_provider::triton::infer::transform_response as infer_response;
use betterlite_provider::triton::stream::decode_stream_line;
use libfuzzer_sys::fuzz_target;

struct WordsCounter;

impl TextTokenCounter for WordsCounter {
    fn count_text(&self, _model: &str, text: &str) -> Option<usize> {
        Some(text.split_whitespace().count())
    }
}

fuzz_target!(|bytes: &[u8]| {
    let _generated = generate_response(200, bytes);
    let _inferred = infer_response(200, bytes);
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let _embedding = transform_embedding_response(200, bytes, "model", &request, &WordsCounter);
    if let Ok(line) = str::from_utf8(bytes) {
        let _chunk = decode_stream_line(line);
    }
});
