use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderName;
use thiserror::Error;

use crate::batches::decode::BatchDecodeError;

#[derive(Debug, Error)]
pub enum BatchesError {
    #[error(
        "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment."
    )]
    NotInitialized,
    #[error(
        "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client."
    )]
    NotAsync,
    #[error(
        "OpenAI client is not an instance of OpenAI. Make sure you passed a sync OpenAI client."
    )]
    NotSync,
    #[error(transparent)]
    Sdk {
        #[from]
        source: SdkError,
    },
    #[error("the SDK answer is not a batch: {source}")]
    Invalid {
        #[from]
        source: BatchDecodeError,
    },
}

// What the OpenAI SDK raises; the lifecycle wrappers hand it on unchanged
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("{message}")]
    Status {
        status: u16,
        error_type: Option<String>,
        param: Option<String>,
        code: Option<String>,
        message: String,
    },
    #[error("the timeout must be a finite, non-negative number of seconds, not {seconds}")]
    InvalidTimeout { seconds: f64 },
    #[error(
        "the shared transport cannot apply a per-phase httpx.Timeout (set phases: {phases}); it enforces one total deadline"
    )]
    UnsupportedTimeoutPhase { phases: String },
    #[error("the request could not be sent: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the batch id cannot be put in a URL path: {source}")]
    Path {
        #[source]
        source: HttpError,
    },
    #[error("{name} is not a valid header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer is not the expected JSON: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the base URL is not usable: {source}")]
    BaseUrl {
        #[source]
        source: url::ParseError,
    },
}
