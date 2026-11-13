use betterlite_context::local_ttl_cache::CacheError;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use http::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatsonxError {
    #[error("{message}")]
    Api {
        status_code: u16,
        message: String,
        headers: Option<HeaderMap>,
    },
    #[error("API key is required")]
    IamApiKeyRequired,
    #[error("{field} must be a string")]
    InvalidField { field: &'static str },
    #[error("the {header} header value is not valid")]
    InvalidHeader {
        header: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the IAM token URL is not valid")]
    IamUrl(#[source] url::ParseError),
    #[error("the IAM token request failed with status {status}")]
    IamStatus { status: u16, headers: HeaderMap },
    #[error("the IAM token request failed")]
    IamTransport(#[source] TransportError),
    #[error("the IAM token reply is not valid")]
    IamReply(#[source] serde_json::Error),
    #[error("the IAM token reply is missing {field}")]
    IamReplyField { field: &'static str },
    #[error("the IAM token cache failed")]
    Cache(#[source] CacheError),
}
