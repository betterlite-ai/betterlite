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
    // LiteLLM's HTTP handler raises for a reply outside 2xx before anything reads its body
    #[error("{body}")]
    Status { status: u16, body: String },
}

impl ImageError {
    // the Python class and HTTP status LiteLLM's caller sees for this failure
    #[must_use]
    pub fn classification(&self) -> (&str, Option<u16>) {
        match self {
            Self::MaxRetries => ("AzureOpenAIError", Some(422)),
            Self::ExtraBody | Self::Field { .. } | Self::Header { .. } => ("TypeError", None),
            Self::Url(_) => ("ValueError", None),
            Self::Client(error) => (error.class(), error.status()),
            Self::Classified(error) => (error.class.as_str(), error.status),
            // the transport errors that carry an HTTP status are the ones LiteLLM's request helper raises itself
            Self::Transport(error) => (
                error
                    .status_code()
                    .map_or("Exception", |_| "AzureOpenAIError"),
                error.status_code(),
            ),
            Self::Reply(ImageReplyError::ResponseObject { status, .. }) => {
                ("Exception", Some(*status))
            }
            Self::Reply(_) | Self::Json(_) | Self::Logging(_) => ("Exception", None),
            Self::Azure { status, .. } => ("AzureOpenAIError", Some(*status)),
            Self::Status { status, .. } => ("MaskedHTTPStatusError", Some(*status)),
        }
    }

    // LiteLLM re-raises its own image errors and wraps every other one with str(error) and status or 500
    #[must_use]
    pub fn into_sync(self) -> Self {
        let (class, status) = self.classification();
        if class == "AzureOpenAIError" {
            return self;
        }
        let status = status.unwrap_or(500);
        let message = self.to_string();
        Self::Azure { status, message }
    }
}
