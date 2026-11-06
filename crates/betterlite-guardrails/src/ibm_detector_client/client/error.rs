use betterlite_http::proxy::limits::ReadError;
use betterlite_http::tls::TlsError;
use reqwest::StatusCode;
use reqwest::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IbmClientError {
    #[error("the IBM detector client could not be built")]
    Tls(#[from] TlsError),
    #[error("the IBM detector client could not be built")]
    Build(#[source] reqwest::Error),
    #[error("the IBM detector request could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the IBM detector extra header name {name} is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the IBM detector header value for {name} is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the IBM detector request failed")]
    Request(#[source] reqwest::Error),
    #[error("the IBM detector answered with HTTP status {status}")]
    Status { status: StatusCode },
    #[error("the IBM detector response could not be read")]
    Read(#[source] ReadError),
    #[error("the IBM detector response was not the expected shape")]
    ResponseJson(#[source] serde_json::Error),
}
