use std::str::Utf8Error;

use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

// Mirrors LiteLLM's CiscoAIDefenseGuardrailAPIError, raised for every transport failure.
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("the Cisco AI Defense API key header value is not valid")]
    InvalidApiKeyHeader { source: InvalidHeaderValue },
    #[error("the Cisco AI Defense user agent header value is not valid")]
    InvalidUserAgentHeader { source: InvalidHeaderValue },
    #[error("Cisco AI Defense {surface} API call timed out after {timeout}s")]
    Timeout {
        surface: String,
        timeout: String,
        source: reqwest::Error,
    },
    #[error("Cisco AI Defense {surface} API request failed: {source}")]
    Request {
        surface: String,
        source: reqwest::Error,
    },
    #[error("Cisco AI Defense {surface} API returned HTTP {status}: {snippet}")]
    HttpStatus {
        surface: String,
        status: u16,
        snippet: String,
    },
    #[error("Cisco AI Defense {surface} API returned HTTP {status} with invalid UTF-8")]
    HttpStatusEncoding {
        surface: String,
        status: u16,
        source: Utf8Error,
    },
    #[error("Cisco AI Defense {surface} API answer exceeds {limit} bytes")]
    ResponseTooLarge { surface: String, limit: usize },
    #[error("Cisco AI Defense {surface} API returned a non-JSON response")]
    ResponseJson {
        surface: String,
        source: serde_json::Error,
    },
}

mod tests;
