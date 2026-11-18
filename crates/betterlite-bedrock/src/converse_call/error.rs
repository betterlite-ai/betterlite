use betterlite_core::api::anthropic_beta_filter::error::BetaFilterError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::bedrock_http_stream::error::HttpStreamError;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::converse_request::error::ConverseRequestError;
use crate::converse_response::transform::TransformError;
use crate::rerank::transport::TransferError;
use crate::signing::error::SigningError;

// The failures completion() can raise: the source error types stay themselves, and only the buffered
// status reply and the timeout become a BedrockError here.
#[derive(Debug, Error)]
pub enum ConverseCallError {
    #[error("the {field} parameter has the wrong type")]
    Param { field: &'static str },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Beta(#[from] BetaFilterError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error("the signed header {name} is not a valid HTTP header")]
    HeaderValue {
        name: String,
        #[source]
        source: http::Error,
    },
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Request(#[from] ConverseRequestError),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error(transparent)]
    Stream(#[from] HttpStreamError),
    #[error(transparent)]
    Transport(#[from] TransferError),
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
}

impl ConverseCallError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Provider { status, .. } => Some(*status),
            Self::Stream(HttpStreamError::Provider { status, .. }) => Some(*status),
            Self::Request(error) => error.status_code(),
            Self::Transform(error) => error.status(),
            Self::Param { .. }
            | Self::Json(_)
            | Self::Beta(_)
            | Self::Signing(_)
            | Self::HeaderValue { .. }
            | Self::Logging(_)
            | Self::Header(_)
            | Self::Transport(_)
            | Self::Stream(_) => None,
        }
    }
}
