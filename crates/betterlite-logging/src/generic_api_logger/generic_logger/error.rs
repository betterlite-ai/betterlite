use super::super::v1_payload::V1PayloadError;
use crate::private_json::error::PrivateJsonError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("the response is not valid JSON")]
    Payload(#[source] serde_json::Error),
    #[error("the batch could not be written as JSON")]
    Dump(#[from] PrivateJsonError),
    #[error("the endpoint is not a valid URL")]
    Url(#[source] url::ParseError),
    #[error("the header name {name} is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header holds a character a header cannot carry")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the request timed out")]
    Timeout,
    #[error("the request could not be completed")]
    Transport(#[source] TransportError),
    #[error("the destination answered with status {status}")]
    Status { status: u16 },
    #[error(transparent)]
    V1(#[from] V1PayloadError),
}
impl ApiError {
    // LiteLLM retries a timeout, an httpx transport error and a 5xx, and nothing else
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Timeout => true,
            Self::Transport(source) => matches!(
                source,
                TransportError::Connect(_)
                    | TransportError::ConnectTimeout(_)
                    | TransportError::Deadline(_)
                    | TransportError::Request(_)
                    | TransportError::Body(_)
            ),
            Self::Status { status } => *status >= 500,
            Self::Payload(_)
            | Self::Dump(_)
            | Self::Url(_)
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::V1(_) => false,
        }
    }
}
