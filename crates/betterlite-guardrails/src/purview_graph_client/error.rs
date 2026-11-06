use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::ReadError;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PurviewGraphError {
    #[error("the Entra user id cannot be used in a Graph path")]
    UserId { source: HttpError },
    #[error("the Graph request header {name} is not valid")]
    Header {
        name: &'static str,
        source: InvalidHeaderValue,
    },
    #[error("the Graph request failed")]
    Request { source: reqwest::Error },
    #[error("the Entra token endpoint answered with status {status}")]
    TokenStatus { status: StatusCode },
    #[error("the Graph API answered with status {status}")]
    Status {
        status: StatusCode,
        retry_after: Option<String>,
    },
    #[error("the Graph answer body could not be read")]
    Body {
        #[source]
        source: ReadError,
    },
    #[error("the Graph answer is not the expected JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the Entra token expires too far in the future to track")]
    TokenExpiry,
}
