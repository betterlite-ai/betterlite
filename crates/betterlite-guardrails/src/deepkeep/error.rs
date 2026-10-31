use std::fmt;

use reqwest::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

#[derive(Error)]
pub enum DeepKeepError {
    #[error("DeepKeep blocked the request: {reason}")]
    Blocked { reason: String },
    #[error("DeepKeep API returned HTTP {status}: {body}")]
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("DeepKeep response was invalid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("DeepKeep response exceeds {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("DeepKeep request failed: {source}")]
    Request { source: reqwest::Error },
    #[error("DeepKeep request timed out: {source}")]
    Timeout { source: reqwest::Error },
    #[error("DeepKeep request could not be built: {source}")]
    RequestBuild { source: reqwest::Error },
    #[error("DeepKeep header {name} is invalid: {source}")]
    InvalidHeader {
        name: &'static str,
        source: InvalidHeaderValue,
    },
    #[error("DeepKeep header name {name} is invalid: {source}")]
    InvalidHeaderName {
        name: String,
        source: InvalidHeaderName,
    },
}

impl fmt::Debug for DeepKeepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Parser errors can quote credentials from an upstream response.
        fmt::Display::fmt(self, formatter)
    }
}
