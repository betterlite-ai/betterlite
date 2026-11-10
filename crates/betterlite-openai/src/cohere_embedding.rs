use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use secrecy::SecretString;
use thiserror::Error;

pub mod request;
pub mod response;
pub mod usage;

mod tests;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["encoding_format", "dimensions"];
pub const DEFAULT_EMBEDDING_INPUT_TYPE: &str = "search_document";
// get_complete_url returns a supplied complete URL unchanged and never appends the suffix itself
pub const DEFAULT_COMPLETE_URL: &str = "https://api.cohere.ai/v2/embed";

#[derive(Debug, Error)]
pub enum CohereEmbeddingError {
    #[error("the embeddings input must be a list of strings")]
    InputNotText,
    #[error("the embeddings input list is empty")]
    EmptyInput,
    #[error("the Cohere embedding JSON cannot be read")]
    Json(#[from] serde_json::Error),
    #[error("the Cohere embedding token usage exceeds the supported range")]
    UsageOverflow,
    #[error("the text could not be encoded")]
    Encoding(#[from] EngineError),
}

// LiteLLM measures embeddings input with litellm.encoding; callers inject the resolved encoding service
pub trait TextEncoding {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError>;
}

impl TextEncoding for Engine {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        self.encode(text).map(|ids| ids.len())
    }
}

pub struct PostCall<'request> {
    pub input: &'request [String],
    pub api_key: Option<&'request SecretString>,
    pub complete_input_dict: &'request Object,
    pub original_response: &'request Json,
}

// input is logging.model_call_details["input"], the one detail the response transform reads
pub trait EmbeddingLogging {
    fn input(&self) -> &[String];
    fn post_call(&mut self, call: PostCall<'_>);
}
