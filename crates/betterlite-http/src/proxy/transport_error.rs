use thiserror::Error;

use crate::error::HttpError;
use crate::proxy::limits::{HeadersTooLarge, ReadError};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error(transparent)]
    Rejected(HttpError),
    #[error("the request could not be built")]
    Build(#[source] reqwest::Error),
    #[error("the connection could not be made")]
    Connect(#[source] reqwest::Error),
    #[error("the connection was not made within the connect timeout")]
    ConnectTimeout(#[source] reqwest::Error),
    #[error("the request ran past its read timeout or total deadline")]
    Deadline(#[source] reqwest::Error),
    #[error("the request failed")]
    Request(#[source] reqwest::Error),
    #[error("the response body failed")]
    Body(#[source] reqwest::Error),
    #[error("the response headers are larger than {limit} bytes")]
    HeadersTooLarge { limit: usize },
    #[error("the response body is larger than {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error("the {name} header holds a character a header cannot carry")]
    InvalidHeader { name: String },
    #[error("the multipart part {name} holds a quote or a line break")]
    MultipartName { name: String },
    #[error("a multipart part contains the boundary {boundary}")]
    MultipartBoundary { boundary: String },
    #[error("the client is shutting down and takes no new requests")]
    Closed,
    #[error("{remaining} requests were still running at the drain deadline")]
    DrainTimeout { remaining: usize },
}

impl TransportError {
    #[must_use]
    pub fn sending(error: reqwest::Error) -> Self {
        if error.is_timeout() && error.is_connect() {
            Self::ConnectTimeout(error)
        } else if error.is_timeout() {
            Self::Deadline(error)
        } else if error.is_connect() {
            Self::Connect(error)
        } else {
            Self::Request(error)
        }
    }

    #[must_use]
    pub fn reading(failure: ReadError) -> Self {
        match failure {
            ReadError::TooLarge { limit } => Self::BodyTooLarge { limit },
            ReadError::Failed(source) if source.is_timeout() => Self::Deadline(source),
            ReadError::Failed(source) => Self::Body(source),
        }
    }
}

impl From<HeadersTooLarge> for TransportError {
    fn from(exceeded: HeadersTooLarge) -> Self {
        Self::HeadersTooLarge {
            limit: exceeded.limit,
        }
    }
}
