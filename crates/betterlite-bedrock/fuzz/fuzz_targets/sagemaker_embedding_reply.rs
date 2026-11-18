#![no_main]
use betterlite_bedrock::sagemaker_embedding::cohere;
use betterlite_bedrock::sagemaker_embedding::hf;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::engine::error::EngineError;
use libfuzzer_sys::fuzz_target;

struct Chars;

impl TextEncoding for Chars {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
    }
}

fuzz_target!(|bytes: &[u8]| {
    let request = Object::default();
    drop(hf::transform_response(
        bytes,
        200,
        "meta-llama/Llama-2-7b",
        &request,
    ));
    drop(cohere::transform_response(
        bytes,
        "cohere.embed-english-v3",
        None,
        &request,
        &Chars,
    ));
});
