use reqwest::StatusCode;
use thiserror::Error;

use super::TRANSIENT_STATUS_CODES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("the Vigil Guard API answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Vigil Guard API could not be reached")]
    Connect,
    #[error("the Vigil Guard API connection timed out")]
    ConnectTimeout,
    #[error("the Vigil Guard API read timed out")]
    ReadTimeout,
    #[error("the Vigil Guard API closed the connection unexpectedly")]
    RemoteProtocol,
    #[error("the Vigil Guard API did not answer in time")]
    Timeout,
    #[error("the Vigil Guard API request failed")]
    Other,
}

impl TransportError {
    #[must_use]
    pub fn is_transient(&self) -> bool {
        if let Self::HttpStatus { status } = self {
            return TRANSIENT_STATUS_CODES.contains(status);
        }
        matches!(
            self,
            Self::Connect
                | Self::ConnectTimeout
                | Self::ReadTimeout
                | Self::RemoteProtocol
                | Self::Timeout
        )
    }
}
