use betterlite_prompts::arguments::json::Json;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use secrecy::SecretString;
use thiserror::Error;

pub mod request;
pub mod response;
pub mod usage;

mod tests;

// get_supported_openai_params returns these two names for every v2 Cohere embedding model
pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["encoding_format", "dimensions"];
// litellm.__init__.COHERE_DEFAULT_EMBEDDING_INPUT_TYPE, the value every text request carries
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
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Cohere embedding field {name} {problem}")]
    Field {
        name: &'static str,
        problem: &'static str,
    },
    #[error("the Cohere embedding token usage exceeds the supported range")]
    UsageOverflow,
    #[error("the text could not be encoded")]
    Encoding {
        #[source]
        source: EngineError,
    },
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

// the part of LiteLLM's logging object the response transform reads
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCallDetails {
    pub input: Vec<String>,
}

pub struct PostCall<'request> {
    pub input: &'request [String],
    pub api_key: Option<&'request SecretString>,
    pub complete_input_dict: &'request request::CohereEmbeddingRequest,
    pub original_response: &'request Json,
}

pub trait EmbeddingLogging {
    fn model_call_details(&self) -> &ModelCallDetails;
    fn post_call(&mut self, call: PostCall<'_>);
}
