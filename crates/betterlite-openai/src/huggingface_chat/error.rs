use crate::chat_content::ChatContentError;
use crate::openai_base_config::Headers;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct HuggingfaceError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> HuggingfaceError {
    HuggingfaceError {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}

#[derive(Debug, Error)]
pub enum HuggingfaceChatError {
    #[error(transparent)]
    Huggingface(#[from] HuggingfaceError),
    #[error("the model {model} has no provider prefix")]
    MissingProviderPrefix { model: String },
    #[error("the provider mapping response has no inferenceProviderMapping field")]
    MappingMissing,
    #[error("no provider mapping found for model {model}")]
    NoProviderMapping { model: String },
    #[error("the provider mapping response is larger than {limit} bytes")]
    MappingTooLarge { limit: usize },
    #[error("the provider mapping response is not valid JSON")]
    MappingDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error("the transformed messages could not be serialized")]
    MessageEncode {
        #[source]
        source: serde_json::Error,
    },
}
