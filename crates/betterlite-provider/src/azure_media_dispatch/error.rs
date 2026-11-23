use betterlite_azure::azure_urls::AzureUrlError;
use betterlite_azure::credentials::selection::AzureTokenError;
use betterlite_azure::image_transport::error::ImageTransportError;
use betterlite_azure::images::error::ImageError;
use betterlite_azure::speech::error::AzureSpeechError;
use betterlite_azure::transcription::call::AzureTranscriptionError;
use betterlite_azure::transcription::native::NativeError;
use betterlite_openai::image_edits::error::ImageEditError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AzureMediaError {
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error(transparent)]
    Edit(#[from] ImageEditError),
    #[error(transparent)]
    EditAuth(#[from] AzureTokenError),
    #[error(transparent)]
    EditUrl(#[from] AzureUrlError),
    #[error(transparent)]
    Transport(#[from] ImageTransportError),
    #[error(transparent)]
    Transcription(#[from] AzureTranscriptionError),
    #[error(transparent)]
    Native(#[from] NativeError),
    #[error(transparent)]
    Speech(#[from] AzureSpeechError),
    // BaseLLMException: the handler raises the upstream status with the reply text before any transform reads the body
    #[error("{message}")]
    EditStatus { status: u16, message: String },
    #[error("the image edit header name `{name}` is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the image edit header `{name}` cannot carry its value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the {field} form field is not a string, a number or a boolean")]
    FormField { field: String },
    #[error("the {operation} operation is not available through the Azure media adapter yet")]
    Unavailable { operation: &'static str },
    #[error("betterlite does not run this operation through the Azure media adapter")]
    UnsupportedOperation,
    #[error("the native transcription request is not raw audio bytes")]
    NativeRequest,
    #[error("the Azure media request URL is not valid")]
    Url(#[source] url::ParseError),
}

impl AzureMediaError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Image(error) => error.classification().1,
            Self::Edit(error) => error.status(),
            Self::EditAuth(error) => error.status(),
            Self::Transport(error) => error.status_code(),
            Self::Transcription(error) => error.status(),
            Self::Native(error) => error.status(),
            Self::Speech(error) => {
                let (_class, status) = error.kind();
                status
            }
            Self::EditStatus { status, .. } => Some(*status),
            Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::FormField { .. }
            | Self::Unavailable { .. }
            | Self::UnsupportedOperation
            | Self::NativeRequest
            | Self::EditUrl(_)
            | Self::Url(_) => None,
        }
    }
}
