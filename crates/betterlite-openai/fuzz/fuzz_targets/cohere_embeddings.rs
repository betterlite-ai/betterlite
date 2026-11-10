#![no_main]

use betterlite_openai::cohere_embedding::request::transform_request;
use betterlite_openai::cohere_embedding::response::{DecodeInput, decode_cohere_embeddings};
use betterlite_openai::cohere_embedding::{
    EmbeddingLogging, ModelCallDetails, PostCall, TextEncoding,
};
use betterlite_tokenizer::engine::error::EngineError;
use libfuzzer_sys::fuzz_target;

struct Counting;

impl TextEncoding for Counting {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.len())
    }
}

struct Silent {
    details: ModelCallDetails,
}

impl EmbeddingLogging for Silent {
    fn model_call_details(&self) -> &ModelCallDetails {
        &self.details
    }

    fn post_call(&mut self, _call: PostCall<'_>) {}
}

fuzz_target!(|bytes: &[u8]| {
    let input = [String::from("hello")];
    let Ok(empty) = serde_json::from_str("{}") else {
        return;
    };
    let Ok(request) = transform_request("m", &input, &empty) else {
        return;
    };
    let encoding = Counting;
    let mut logging = Silent {
        details: ModelCallDetails {
            input: input.to_vec(),
        },
    };
    let _unused = decode_cohere_embeddings(
        DecodeInput {
            body: bytes,
            model: "m",
            input: &input,
            api_key: None,
            request_data: &request,
            encoding: &encoding,
        },
        &mut logging,
    );
});
