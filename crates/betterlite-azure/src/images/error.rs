use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::image_generation::reply::ImageReplyError;
use thiserror::Error;

use crate::azure_urls::AzureUrlError;
use crate::credentials::source::ProviderFailure;
use crate::endpoints::initialize::EndpointError;
use crate::image_transport::error::ImageTransportError;

// An error another ported layer classifies itself
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ClassifiedError {
    pub class: String,
    pub status: Option<u16>,
    pub message: String,
}

impl From<ProviderFailure> for ClassifiedError {
    fn from(failure: ProviderFailure) -> Self {
        Self {
            class: failure.class,
            status: None,
            message: failure.message,
        }
    }
}

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("max retries must be an int")]
    MaxRetries,
    #[error("extra_body must be an object")]
    ExtraBody,
    #[error("{field} must be a string")]
    Field { field: &'static str },
    #[error("the image headers are not valid: {message}")]
    Header { message: String },
    #[error(transparent)]
    Url(#[from] AzureUrlError),
    #[error(transparent)]
    Client(#[from] EndpointError),
    #[error(transparent)]
    Classified(#[from] ClassifiedError),
    #[error(transparent)]
    Transport(#[from] ImageTransportError),
    #[error(transparent)]
    Reply(#[from] ImageReplyError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("{message}")]
    Azure { status: u16, message: String },
}

impl ImageError {
    // the Python class LiteLLM's caller sees for this failure
    #[must_use]
    pub fn class(&self) -> &str {
        match self {
            Self::MaxRetries | Self::Azure { .. } => "AzureOpenAIError",
            Self::ExtraBody | Self::Field { .. } | Self::Header { .. } => "TypeError",
            Self::Url(_) => "ValueError",
            Self::Client(error) => error.class(),
            Self::Classified(error) => &error.class,
            // the transport errors that carry an HTTP status are the ones LiteLLM's request helper raises itself
            Self::Transport(error) if error.status_code().is_some() => "AzureOpenAIError",
            Self::Transport(_) | Self::Reply(_) | Self::Json(_) | Self::Logging(_) => "Exception",
        }
    }

    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::MaxRetries => Some(422),
            Self::Url(_) | Self::ExtraBody | Self::Field { .. } | Self::Header { .. } => None,
            Self::Json(_) | Self::Logging(_) | Self::Reply(_) => None,
            Self::Client(error) => error.status(),
            Self::Classified(error) => error.status,
            Self::Transport(error) => error.status_code(),
            Self::Reply(ImageReplyError::ResponseObject { status, .. }) => Some(*status),
            Self::Azure { status, .. } => Some(*status),
        }
    }

    // LiteLLM re-raises its own image errors and wraps every other one with str(error) and status or 500
    #[must_use]
    pub fn into_sync(self) -> Self {
        if self.class() == "AzureOpenAIError" {
            return self;
        }
        let status = self.status().unwrap_or(500);
        let message = self.to_string();
        Self::Azure { status, message }
    }
}
