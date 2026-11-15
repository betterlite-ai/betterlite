use std::time::TryFromFloatSecsError;

use betterlite_http::tls::TlsError;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CatoError {
    #[error(
        "no Cato Networks API key: set CATO_API_KEY or pass api_key to the guardrail configuration"
    )]
    MissingApiKey,
    #[error("the {name} header value for Cato Networks is not valid")]
    InvalidHeader {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Cato Networks API request timed out")]
    Timeout {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Cato Networks API could not be reached")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Cato Networks API answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Cato Networks API answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Cato Networks API answer is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    Blocked { message: String },
    #[error(
        "the Cato Networks analyze answer does not match the inspected request, so the request cannot be rewritten without forwarding unredacted text"
    )]
    RedactionMismatch,
    #[error("the Cato Networks TLS settings are not valid")]
    Tls {
        #[source]
        source: TlsError,
    },
    #[error("the Cato Networks timeout of {seconds} seconds is not a valid duration")]
    TimeoutSetting {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the Cato Networks API client could not be built")]
    Client {
        #[source]
        source: reqwest::Error,
    },
}

impl CatoError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } | Self::RedactionMismatch => StatusCode::BAD_REQUEST,
            Self::Timeout { .. }
            | Self::Request { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => StatusCode::BAD_GATEWAY,
            Self::HttpStatus { status } => *status,
            Self::MissingApiKey
            | Self::InvalidHeader { .. }
            | Self::Tls { .. }
            | Self::TimeoutSetting { .. }
            | Self::Client { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub const INVALID_REQUEST_ERROR: &str = "invalid_request_error";
