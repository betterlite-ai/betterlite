use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::batch_records::BatchRecordsError;

#[derive(Debug, Error)]
pub enum XaiBatchesError {
    #[error("Missing xAI API Key. Pass api_key or set XAI_API_KEY")]
    MissingApiKey,
    #[error("input_file_id is required to create an xAI batch")]
    InputFileRequired,
    #[error("the xAI batch API answered with status {status}")]
    Status {
        status: u16,
        message: String,
        headers: IndexMap<String, String>,
    },
    #[error(transparent)]
    Limit(#[from] BatchRecordsError),
    #[error("the xAI batch answer is not the expected JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the xAI batch request could not be sent: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the xAI batch id cannot be put in a URL path: {source}")]
    Path {
        #[source]
        source: HttpError,
    },
    #[error("the authorization header cannot carry the API key: {source}")]
    Authorization {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the xAI API base is not a URL: {source}")]
    BaseUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("the xAI batch answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl XaiBatchesError {
    // LiteLLM's BaseLLMException carries a status; a local failure has none
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey => Some(401),
            Self::InputFileRequired => Some(400),
            Self::Status { status, .. } => Some(*status),
            Self::Limit(_)
            | Self::Decode { .. }
            | Self::Transport { .. }
            | Self::Path { .. }
            | Self::Authorization { .. }
            | Self::BaseUrl { .. }
            | Self::Encode { .. } => None,
        }
    }
}
