use std::str::Utf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::analytics::python::FormatError;
use betterlite_prompts::error::FactoryError;
use betterlite_tokenizer::count::error::CountError;
use http::HeaderMap;
use http::header::InvalidHeaderValue;
use thiserror::Error;

// a rejection Replicate itself answered, with the status, text and headers LiteLLM's ReplicateError carries
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ReplicateError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<HeaderMap>,
}

#[derive(Debug, Error)]
pub enum ReplicateFailure {
    #[error(transparent)]
    Api(Box<ReplicateError>),
    #[error(transparent)]
    Prompt(Box<FactoryError>),
    #[error("the messages cannot be read as prompt messages")]
    Messages {
        #[source]
        source: serde_json::Error,
    },
    #[error("supports_system_prompt must be a boolean")]
    SupportsSystemPrompt,
    #[error("the Replicate request body cannot be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Replicate reply is not JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Replicate reply is not a JSON object")]
    ReplyNotObject,
    #[error("the Replicate reply has no status")]
    MissingStatus,
    #[error("urls.get in the Replicate reply is not a string")]
    PredictionUrlType,
    #[error("the Replicate reply body is not UTF-8")]
    BodyNotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the Replicate request failed")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the Replicate header cannot be built")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Replicate reply text cannot be formatted")]
    Format {
        #[source]
        source: FormatError,
    },
    #[error("the tokens of the Replicate call cannot be counted")]
    TokenCount {
        #[source]
        source: CountError,
    },
    #[error("the token usage overflows")]
    UsageOverflow,
}

impl From<ReplicateError> for ReplicateFailure {
    fn from(error: ReplicateError) -> Self {
        Self::Api(Box::new(error))
    }
}

impl From<FactoryError> for ReplicateFailure {
    fn from(error: FactoryError) -> Self {
        Self::Prompt(Box::new(error))
    }
}

impl ReplicateFailure {
    #[must_use]
    pub const fn api(&self) -> Option<&ReplicateError> {
        match self {
            Self::Api(error) => Some(error),
            Self::Prompt(_)
            | Self::Messages { .. }
            | Self::SupportsSystemPrompt
            | Self::Encode { .. }
            | Self::Decode { .. }
            | Self::ReplyNotObject
            | Self::MissingStatus
            | Self::PredictionUrlType
            | Self::BodyNotUtf8 { .. }
            | Self::Transport { .. }
            | Self::Header { .. }
            | Self::Format { .. }
            | Self::TokenCount { .. }
            | Self::UsageOverflow => None,
        }
    }
}

mod tests;
