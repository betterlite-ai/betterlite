use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
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
    #[error("{message}")]
    Status {
        status: u16,
        message: String,
        headers: Option<ImageHeaders>,
    },
    #[error("{0}")]
    Reply(#[from] ImageReplyError),
    #[error("a request header name or value is not valid")]
    Header(#[source] http::Error),
}

impl ImageVariationSource {
    // the one place deciding what a failure contributes to LiteLLM's OpenAIError wrapper
    fn parts(&self) -> (Option<u16>, Option<ImageHeaders>) {
        match self {
            Self::Status {
                status, headers, ..
            } => (Some(*status), headers.clone()),
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
            | Self::Header(_) => (None, None),
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
        let (status, headers) = source.parts();
        Self {
            status_code: status.unwrap_or(500),
            message: source.to_string(),
            headers,
            source: Box::new(source),
        }
    }
}

pub type ImageVariationResult<T> = Result<T, OpenAiImageVariationError>;
