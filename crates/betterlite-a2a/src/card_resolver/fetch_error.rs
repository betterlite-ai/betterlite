use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CardFetchError {
    #[error("the card URL is not valid: {0}")]
    InvalidUrl(#[source] url::ParseError),
    #[error("the request could not be sent: {0}")]
    Transport(#[source] TransportError),
    #[error("HTTP {status}")]
    Status { status: u16 },
    #[error("the answer is not an agent card: {0}")]
    Parse(#[source] serde_json::Error),
}

impl CardFetchError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Status { status } => Some(*status),
            Self::InvalidUrl(_) | Self::Transport(_) | Self::Parse(_) => None,
        }
    }
}

#[derive(Debug)]
pub struct ProbeFailure {
    pub path: String,
    pub reason: CardFetchError,
}
