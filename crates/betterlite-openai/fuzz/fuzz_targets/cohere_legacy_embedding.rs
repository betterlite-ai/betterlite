#![no_main]

use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::{
    DecodeInput, decode_cohere_legacy_embedding,
};
use betterlite_openai::cohere_legacy_embedding::{EmbeddingHooks, PostCall, PreCall};
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::engine::error::EngineError;
use libfuzzer_sys::fuzz_target;

struct Fixture;
impl TextEncoding for Fixture {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.len())
    }
}
impl EmbeddingHooks for Fixture {
    fn pre_call(&mut self, _call: PreCall<'_>) {}

    fn post_call(&mut self, _call: PostCall<'_>) {}
}

fuzz_target!(|bytes: &[u8]| {
    let input = [String::from("hello")];
    let mut hooks = Fixture;
    let reply = DecodeInput {
        body: bytes,
        model: "m",
        api_key: None,
        request_data: &Object::new(),
        input: &input,
        encoding: &Fixture,
    };
    let _unused = decode_cohere_legacy_embedding(reply, &mut hooks);
});
