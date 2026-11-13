#![no_main]

use betterlite_openai::cohere_embedding::request::transform_request;
use betterlite_openai::cohere_embedding::response::{DecodeInput, transform_embedding_response};
use betterlite_openai::cohere_embedding::{EmbeddingLogging, PostCall, TextEncoding};
use betterlite_tokenizer::engine::error::EngineError;
use libfuzzer_sys::fuzz_target;

struct Fixture(Vec<String>);

impl TextEncoding for Fixture {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.len())
    }
}

impl EmbeddingLogging for Fixture {
    fn input(&self) -> &[String] {
        &self.0
    }

    fn post_call(&mut self, _call: PostCall<'_>) {}
}

fuzz_target!(|bytes: &[u8]| {
    let input = [String::from("hello")];
    let Ok(empty) = serde_json::from_str("{}") else {
        return;
    };
    let request = transform_request("m", &input, &empty);
    let mut fixture = Fixture(input.to_vec());
    let encoding = Fixture(Vec::new());
    let reply = DecodeInput {
        body: bytes,
        model: "m",
        api_key: None,
        request_data: &request,
        encoding: &encoding,
    };
    let _unused = transform_embedding_response(reply, &mut fixture);
});
