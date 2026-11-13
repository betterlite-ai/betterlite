use std::string::FromUtf8Error;

use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AktoError {
    #[error("akto_base_url is required; set AKTO_GUARDRAIL_API_BASE or configure akto_base_url")]
    MissingBaseUrl,
    #[error("akto_api_key is required; set AKTO_API_KEY or configure akto_api_key")]
    MissingApiKey,
    #[error("the Akto API key is not a valid HTTP header value")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the Akto request could not be encoded")]
    Encode { source: serde_json::Error },
    #[error("the Akto service could not be reached")]
    Transport { source: reqwest::Error },
    #[error("the Akto service returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Akto response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Akto response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the Akto response is not valid UTF-8")]
    ResponseEncoding { source: FromUtf8Error },
    #[error("the Akto response field {field} has an unexpected type")]
    ResponseField { field: &'static str },
    #[error("the Akto guardrail service is unreachable")]
    Unreachable,
}

impl AktoError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Unreachable => StatusCode::SERVICE_UNAVAILABLE,
            Self::MissingBaseUrl
            | Self::MissingApiKey
            | Self::InvalidHeader { .. }
            | Self::Encode { .. }
            | Self::Transport { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::ResponseEncoding { .. }
            | Self::ResponseField { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // LiteLLM hands a RequestError or HTTPStatusError to handle_unreachable and nothing else
    #[must_use]
    pub const fn is_unreachable(&self) -> bool {
        match self {
            Self::Transport { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::ResponseEncoding { .. }
            | Self::ResponseField { .. } => true,
            Self::MissingBaseUrl
            | Self::MissingApiKey
            | Self::InvalidHeader { .. }
            | Self::Encode { .. }
            | Self::Unreachable => false,
        }
    }
}
