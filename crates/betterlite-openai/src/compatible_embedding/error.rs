use std::string::FromUtf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::compatible_wire::error::OpenAiLikeError;

// Display is LiteLLM's str(error), which the async failure log records
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error(transparent)]
    Upstream(#[from] OpenAiLikeError),
    #[error("{0}")]
    PreCall(#[source] PrePostError),
    #[error("{0}")]
    Url(#[source] url::ParseError),
    #[error("{0}")]
    Header(#[source] http::Error),
    #[error("{0}")]
    Encode(#[source] serde_json::Error),
    #[error("Timeout error occurred.")]
    Timeout(#[source] TransportError),
    #[error("{0}")]
    Transport(#[source] TransportError),
    #[error("{text}")]
    Status { status: u16, text: String },
    #[error("the {status} answer is not UTF-8 text: {source}")]
    StatusBody {
        status: u16,
        #[source]
        source: FromUtf8Error,
    },
    #[error("{0}")]
    Reply(#[source] serde_json::Error),
    #[error("{0}")]
    Response(#[source] serde_json::Error),
}

impl EmbeddingError {
    // the status of the OpenAILikeError LiteLLM raises, and none where it raises another type
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Upstream(error) => Some(error.status_code),
            Self::Status { status, .. } | Self::StatusBody { status, .. } => Some(*status),
            Self::Timeout(_) => Some(408),
            Self::Url(_)
            | Self::Header(_)
            | Self::Encode(_)
            | Self::Transport(_)
            | Self::Reply(_) => Some(500),
            Self::PreCall(_) | Self::Response(_) => None,
        }
    }
}
