use std::error::Error as StdError;
use std::string::FromUtf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::compatible_wire::error::{CompatibleWireError, OpenAiLikeError};
use thiserror::Error;

// Display is LiteLLM's str(error): the status text it raises, or the transport failure it wraps
#[derive(Debug, Error)]
pub enum CompatibleChatError {
    #[error(transparent)]
    OpenAiLike(#[from] OpenAiLikeError),
    #[error("the {status} answer is not UTF-8 text: {source}")]
    StatusBody {
        status: u16,
        #[source]
        source: FromUtf8Error,
    },
    #[error(transparent)]
    Wire(#[from] CompatibleWireError),
    #[error("{0}")]
    PreCall(#[source] PrePostError),
    #[error("Timeout error occurred.")]
    Timeout(#[source] TransportError),
    #[error("{0}")]
    Transport(#[source] TransportError),
    #[error("the {field} option must be {expected}")]
    OptionType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("{0}")]
    Url(#[source] url::ParseError),
    #[error("{0}")]
    Header(#[source] http::Error),
    #[error("{0}")]
    Encode(#[source] serde_json::Error),
    #[error("the message transform failed: {0}")]
    Transform(#[source] Box<dyn StdError + Send + Sync>),
}

impl CompatibleChatError {
    // the status of the OpenAILikeError LiteLLM raises, and none where it raises another type
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::OpenAiLike(error) => Some(error.status_code),
            Self::StatusBody { status, .. } => Some(*status),
            Self::Timeout(_) => Some(408),
            Self::OptionType { .. } => Some(400),
            Self::Wire(_)
            | Self::Url(_)
            | Self::Header(_)
            | Self::Encode(_)
            | Self::Transport(_) => Some(500),
            Self::PreCall(_) | Self::Transform(_) => None,
        }
    }
}
