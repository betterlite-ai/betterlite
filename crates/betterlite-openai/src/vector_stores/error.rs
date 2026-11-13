use super::reply::ReplyError;
use betterlite_http::error::HttpError;
use http::{HeaderMap, StatusCode};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VectorStoreError {
    #[error("api_key is required")]
    MissingApiKey,
    #[error("invalid path segment")]
    Segment(#[from] HttpError),
    #[error("invalid vector store reply (status {status})")]
    InvalidReply {
        status: StatusCode,
        headers: Box<HeaderMap>,
        #[source]
        source: Box<ReplyError>,
    },
}
