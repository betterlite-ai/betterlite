use std::str::Utf8Error;

use thiserror::Error;

const SYNTHETIC_URL: &str = "https://docs.codestral.com/user-guide/inference/rest_api";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRequest {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResponse {
    pub status_code: u16,
    pub request: NativeRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct NativeError {
    pub status_code: u16,
    pub message: String,
    pub request: NativeRequest,
    pub response: NativeResponse,
}

impl NativeError {
    #[must_use]
    pub fn new(
        status_code: u16,
        message: String,
        request: Option<NativeRequest>,
        response: Option<NativeResponse>,
    ) -> Self {
        let resolved_request = request.unwrap_or_else(|| NativeRequest {
            method: "POST".to_owned(),
            url: SYNTHETIC_URL.to_owned(),
        });
        let resolved_response = response.unwrap_or_else(|| NativeResponse {
            status_code,
            request: resolved_request.clone(),
        });
        Self {
            status_code,
            message,
            request: resolved_request,
            response: resolved_response,
        }
    }
}

#[derive(Debug, Error)]
pub enum TextCompletionCodestralError {
    #[error("{0}")]
    Native(Box<NativeError>),
    #[error("the codestral record is {size} bytes, over the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("the codestral record is not valid UTF-8")]
    BodyNotUtf8(#[source] Utf8Error),
    #[error("the codestral stream chunk is not a stream chunk object")]
    Stream(#[source] serde_json::Error),
}
