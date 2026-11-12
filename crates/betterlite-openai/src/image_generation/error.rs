use std::string::FromUtf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue, ToStrError};
use thiserror::Error;

use super::reply::ImageReplyError;
use crate::compatible_wire::error::OpenAiLikeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorStatus {
    Missing,
    Code(u16),
}

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
    #[error(
        "Missing credentials. Please pass an `api_key`, `workload_identity`, `admin_api_key`, or set the `OPENAI_API_KEY` or `OPENAI_ADMIN_KEY` environment variable."
    )]
    MissingApiKey,
    #[error("{field} must map names to plain values")]
    Extra { field: &'static str },
    #[error(transparent)]
    RequestHeaderName(#[from] InvalidHeaderName),
    #[error("the {name} request header cannot carry its value")]
    RequestHeader {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the {name} response header is not text")]
    ResponseHeader {
        name: String,
        #[source]
        source: ToStrError,
    },
    #[error(transparent)]
    Reply(#[from] ImageReplyError),
}

impl ErrorStatus {
    #[must_use]
    pub const fn code(self) -> Option<u16> {
        match self {
            Self::Code(code) => Some(code),
            Self::Missing => None,
        }
    }
}

impl ImageGenerationSource {
    #[must_use]
    pub const fn status(&self) -> ErrorStatus {
        match self {
            Self::Upstream(error) => ErrorStatus::Code(error.status_code),
            Self::ResponseObject { status, .. } => ErrorStatus::Code(*status),
            Self::MaxRetries => ErrorStatus::Code(422),
            Self::Transport(_)
            | Self::Url(_)
            | Self::Encode(_)
            | Self::MissingApiKey
            | Self::Extra { .. }
            | Self::RequestHeaderName(_)
            | Self::PreCall(_)
            | Self::StatusBody { .. }
            | Self::RequestHeader { .. }
            | Self::ResponseHeader { .. }
            | Self::Reply(_) => ErrorStatus::Missing,
        }
    }
}

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
            found @ ErrorStatus::Code(_) => found,
        };
        Self {
            status,
            message: source.to_string(),
            source: Box::new(source),
        }
    }
}

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
            Self::Wrapped(error) => error.status.code(),
            Self::Raw(source) => source.status().code(),
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
    pub fn failure_source(&self) -> &ImageGenerationSource {
        match self {
            Self::Wrapped(error) => &error.source,
            Self::Raw(source) => source,
        }
    }
}
