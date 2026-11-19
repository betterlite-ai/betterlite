use betterlite_error::error::ErrorClass;
use thiserror::Error;

// The httpx family a provider body read falls into; only the first two take LiteLLM's fallback branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    ReadError,
    RemoteProtocolError,
    HttpError,
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct TransportFailure {
    pub kind: TransportKind,
    pub message: String,
}
impl TransportFailure {
    #[must_use]
    pub fn new(kind: TransportKind, message: &str) -> Self {
        Self {
            kind,
            message: message.to_owned(),
        }
    }
    #[must_use]
    pub fn is_transport(&self) -> bool {
        matches!(
            self.kind,
            TransportKind::ReadError | TransportKind::RemoteProtocolError
        )
    }
}
#[derive(Debug, Error)]
pub enum DriverError {
    #[error("the responses stream failed")]
    Class { class: ErrorClass },
    #[error("{0}")]
    Transport(TransportFailure),
}
