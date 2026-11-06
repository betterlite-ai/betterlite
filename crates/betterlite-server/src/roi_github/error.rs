use axum::http::header::InvalidHeaderValue;
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("Could not reach GitHub. Check the API URL and network connection.")]
    Unreachable,
    #[error("Could not reach GitHub. Check the API URL and network connection.")]
    Transport(#[source] TransportError),
    #[error("Could not reach GitHub. Check the API URL and network connection.")]
    Url(#[source] url::ParseError),
    #[error("Could not reach GitHub. Check the API URL and network connection.")]
    Token(#[source] InvalidHeaderValue),
    #[error("{label} (HTTP {status})")]
    Http { label: &'static str, status: u16 },
    #[error("{message}")]
    Pagination {
        message: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("GitHub's pagination limit was reached. Narrow the date range.")]
    PaginationLimit,
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error(transparent)]
    Pool(#[from] PoolError),
    #[error(transparent)]
    Source(#[from] SourceError),
}
