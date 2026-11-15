use thiserror::Error;

use crate::huggingface_chat::error::HuggingfaceError;
use crate::openai_base_config::Headers;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum HuggingfaceRerankError {
    #[error(transparent)]
    Huggingface(#[from] HuggingfaceError),
    #[error("query is required for HuggingFace rerank")]
    MissingQuery,
    #[error("Cohere 'documents' param is required for HuggingFace rerank")]
    MissingTexts,
    #[error("the rerank response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the rerank response is not a JSON list")]
    ResponseShape,
    #[error("the rerank response item at position {position} is not a rerank item")]
    ResponseItem {
        position: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("the rerank request field {field} holds a value of the wrong type")]
    RequestField { field: &'static str },
    #[error("the document at position {position} has a text that is not a string")]
    DocumentText { position: usize },
    #[error("a token count does not fit the rerank response")]
    TokenOverflow,
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16) -> HuggingfaceError {
    HuggingfaceError {
        status_code,
        message: message.to_owned(),
        headers: Headers::new(),
    }
}
