use std::string::FromUtf8Error;

use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use super::wire::{ErrorDetail, ViolationDetails};

#[derive(Debug, Error)]
pub enum ModerationError {
    #[error(
        "OpenAI Moderation: api_key is required. Set OPENAI_API_KEY environment variable or pass it in configuration."
    )]
    MissingApiKey,
    #[error("the OpenAI Moderation API key is not a valid header value")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the OpenAI Moderation request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the OpenAI Moderation API could not be reached")]
    Request { source: reqwest::Error },
    #[error("OpenAI Moderation API request failed with status {status}")]
    Api { status: StatusCode, body: String },
    #[error(
        "OpenAI Moderation API request failed with status {status} and a body that is not UTF-8"
    )]
    ApiBodyNotUtf8 {
        status: StatusCode,
        source: FromUtf8Error,
    },
    #[error("the OpenAI Moderation response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the OpenAI Moderation response is not a valid moderation object")]
    ResponseJson { source: serde_json::Error },
    #[error("Violated OpenAI moderation policy")]
    Flagged { details: ViolationDetails },
}

impl ModerationError {
    // LiteLLM raises HTTPException(400, ...) on a flagged result and the upstream status on a failed call.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Api { status, .. } | Self::ApiBodyNotUtf8 { status, .. } => *status,
            Self::Flagged { .. } => StatusCode::BAD_REQUEST,
            // AsyncHTTPHandler.post turns httpx.TimeoutException into litellm.Timeout, which carries 408.
            Self::Timeout { .. } => StatusCode::REQUEST_TIMEOUT,
            Self::MissingApiKey
            | Self::InvalidHeader { .. }
            | Self::Request { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // The dict LiteLLM puts in HTTPException.detail; the other variants have none.
    #[must_use]
    pub fn detail(&self) -> Option<ErrorDetail<'_>> {
        match self {
            Self::Api { body, .. } => Some(ErrorDetail {
                error: "OpenAI Moderation API request failed",
                details: Some(body),
                moderation_result: None,
            }),
            Self::Flagged { details } => Some(ErrorDetail {
                error: "Violated OpenAI moderation policy",
                details: None,
                moderation_result: Some(details),
            }),
            Self::MissingApiKey
            | Self::InvalidHeader { .. }
            | Self::ApiBodyNotUtf8 { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => None,
        }
    }
}
