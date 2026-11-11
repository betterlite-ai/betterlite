use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_prompts::builtin::PromptError;
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::media::error::MediaError;
use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

use crate::ollama_chat::error::OllamaError;

#[derive(Debug, Error)]
pub enum OllamaGenerateError {
    #[error(transparent)]
    Chat(#[from] OllamaError),
    #[error("the generate prompt is invalid: {source}")]
    Prompt {
        #[from]
        source: PromptError,
    },
    #[error("the custom prompt is invalid: {source}")]
    Factory {
        #[from]
        source: FactoryError,
    },
    #[error("a remote image could not be inlined: {source}")]
    Media {
        #[from]
        source: MediaError,
    },
    #[error("the token encoder could not count the Ollama prompts: {source}")]
    Encode {
        #[from]
        source: CountError,
    },
    #[error("the Ollama image could not be re-encoded as JPEG: {source}")]
    ImageEncode {
        #[source]
        source: image::ImageError,
    },
    #[error("the message could not be prepared for the Ollama prompt: {source}")]
    Message {
        #[source]
        source: serde_json::Error,
    },
}

impl OllamaGenerateError {
    // LiteLLM answers a malformed prompt or custom-prompt setup with 400 and an encoder failure with a 500
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Chat(error) => error.status(),
            Self::Prompt { .. } | Self::Factory { .. } | Self::Media { .. } => Some(400),
            Self::Encode { .. } | Self::ImageEncode { .. } | Self::Message { .. } => None,
        }
    }
}

impl From<OllamaGenerateError> for ProviderError {
    fn from(error: OllamaGenerateError) -> Self {
        match error {
            OllamaGenerateError::Chat(shared) => Self::from(shared),
            other @ (OllamaGenerateError::Prompt { .. }
            | OllamaGenerateError::Factory { .. }
            | OllamaGenerateError::Media { .. }
            | OllamaGenerateError::Encode { .. }
            | OllamaGenerateError::ImageEncode { .. }
            | OllamaGenerateError::Message { .. }) => {
                let status_code = other.status().unwrap_or(500);
                Self::from(ProviderErrorParts {
                    status_code,
                    message: other.to_string(),
                    headers: None,
                    request: None,
                    response: None,
                    body: None,
                    status_code_is_synthesized: false,
                })
            }
        }
    }
}
