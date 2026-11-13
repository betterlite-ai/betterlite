use std::string::FromUtf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::ToStrError;
use thiserror::Error;

use crate::api::image_contracts::records::ImageHeaders;
use crate::image_generation::reply::ImageReplyError;

#[derive(Debug, Error)]
pub enum ImageVariationSource {
    #[error("image variation provider not found: {provider}")]
    MissingConfig { provider: &'static str },
    #[error("data field is required, for openai image variations")]
    MissingData,
    #[error("max_retries must be an integer")]
    MaxRetries,
    #[error("no API key is set for the image variation call")]
    MissingApiKey,
    #[error("the supplied client is {supplied} but this call needs the {needed} client")]
    ClientKind {
        supplied: &'static str,
        needed: &'static str,
    },
    #[error("the {field} option must be a plain value")]
    Option { field: String },
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the {status} answer is not UTF-8 text")]
    StatusBody {
        status: u16,
        #[source]
        source: FromUtf8Error,
    },
    #[error("{message}")]
    Status {
        status: u16,
        message: String,
        headers: Option<ImageHeaders>,
    },
    #[error("the {name} response header is not text")]
    StatusHeader {
        status: u16,
        name: String,
        #[source]
        source: ToStrError,
    },
    #[error("{0}")]
    Reply(#[from] ImageReplyError),
    #[error("a request header name or value is not valid")]
    Header(#[source] http::Error),
}

impl ImageVariationSource {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. }
            | Self::StatusBody { status, .. }
            | Self::StatusHeader { status, .. } => Some(*status),
            Self::MissingConfig { .. }
            | Self::MissingData
            | Self::MaxRetries
            | Self::MissingApiKey
            | Self::ClientKind { .. }
            | Self::Option { .. }
            | Self::Url(_)
            | Self::Transport(_)
            | Self::Logging(_)
            | Self::Reply(_)
            | Self::Header(_) => None,
        }
    }

    #[must_use]
    pub fn headers(&self) -> Option<ImageHeaders> {
        match self {
            Self::Status { headers, .. } => headers.clone(),
            Self::MissingConfig { .. }
            | Self::MissingData
            | Self::MaxRetries
            | Self::MissingApiKey
            | Self::ClientKind { .. }
            | Self::Option { .. }
            | Self::Url(_)
            | Self::Transport(_)
            | Self::Logging(_)
            | Self::StatusBody { .. }
            | Self::StatusHeader { .. }
            | Self::Reply(_)
            | Self::Header(_) => None,
        }
    }
}

// LiteLLM wraps every failure here as its OpenAIError, so the caller sees one class with a status and headers
#[derive(Debug, Error)]
#[error("{message}")]
pub struct OpenAiImageVariationError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<ImageHeaders>,
    #[source]
    pub source: Box<ImageVariationSource>,
}

impl OpenAiImageVariationError {
    #[must_use]
    pub fn wrapped(source: ImageVariationSource) -> Self {
        let status_code = source.status().unwrap_or(500);
        let headers = source.headers();
        Self {
            status_code,
            message: source.to_string(),
            headers,
            source: Box::new(source),
        }
    }
}

pub type ImageVariationResult<T> = Result<T, OpenAiImageVariationError>;
