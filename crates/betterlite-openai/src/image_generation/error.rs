use std::string::FromUtf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use super::reply::ImageReplyError;
use crate::compatible_wire::error::OpenAiLikeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorStatus {
    Missing,
    Code(u16),
}

// the failures the source methods catch: an OpenAIError is re-raised, everything else is wrapped or rethrown
#[derive(Debug, Error)]
pub enum ImageGenerationSource {
    #[error(transparent)]
    Upstream(#[from] OpenAiLikeError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    PreCall(#[from] PrePostError),
    #[error("the {status} answer is not UTF-8 text")]
    StatusBody {
        status: u16,
        #[source]
        source: FromUtf8Error,
    },
    #[error("{message}")]
    ResponseObject { status: u16, message: String },
    #[error("max_retries must be an integer")]
    MaxRetries,
    #[error("the {name} request header cannot carry its value")]
    RequestHeader {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the {name} response header is not text")]
    ResponseHeader {
        name: String,
        #[source]
        source: http::header::ToStrError,
    },
    #[error(transparent)]
    Reply(#[from] ImageReplyError),
}

impl ImageGenerationSource {
    // the status_code attribute the caught exception carries, absent where it carries none
    #[must_use]
    pub const fn status(&self) -> ErrorStatus {
        match self {
            Self::Upstream(error) => ErrorStatus::Code(error.status_code),
            Self::ResponseObject { status, .. } => ErrorStatus::Code(*status),
            Self::MaxRetries => ErrorStatus::Code(422),
            Self::Transport(_)
            | Self::Url(_)
            | Self::Encode(_)
            | Self::PreCall(_)
            | Self::StatusBody { .. }
            | Self::RequestHeader { .. }
            | Self::ResponseHeader { .. }
            | Self::Reply(_) => ErrorStatus::Missing,
        }
    }
}

// litellm's OpenAIError from common_utils: the status and diagnostic the sync method raises
#[derive(Debug, Error)]
#[error("{message}")]
pub struct OpenAiImageError {
    pub status: ErrorStatus,
    pub message: String,
    #[source]
    pub source: Box<ImageGenerationSource>,
}

impl OpenAiImageError {
    #[must_use]
    pub fn wrapped(source: ImageGenerationSource) -> Self {
        let status = match source.status() {
            ErrorStatus::Missing => ErrorStatus::Code(500),
            found => found,
        };
        Self {
            status,
            message: source.to_string(),
            source: Box::new(source),
        }
    }
}

// the sync method wraps every failure, and the delegated async method rethrows the original
#[derive(Debug, Error)]
pub enum ImageGenerationFailure {
    #[error(transparent)]
    Wrapped(#[from] OpenAiImageError),
    #[error(transparent)]
    Raw(#[from] ImageGenerationSource),
}

impl ImageGenerationFailure {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Wrapped(error) => match error.status {
                ErrorStatus::Code(status) => Some(status),
                ErrorStatus::Missing => None,
            },
            Self::Raw(source) => match source.status() {
                ErrorStatus::Code(status) => Some(status),
                ErrorStatus::Missing => None,
            },
        }
    }

    #[must_use]
    pub const fn mapped(&self) -> bool {
        match self {
            Self::Wrapped(_) => true,
            Self::Raw(_) => false,
        }
    }

    #[must_use]
    pub fn source(&self) -> &ImageGenerationSource {
        match self {
            Self::Wrapped(error) => &error.source,
            Self::Raw(source) => source,
        }
    }
}
