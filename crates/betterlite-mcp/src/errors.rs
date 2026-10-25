use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SessionTokenError {
    #[error("session token is too large: {size_bytes} bytes, maximum {max_bytes}")]
    TooLarge { size_bytes: usize, max_bytes: usize },
    #[error("candidate is not a session token")]
    NotASessionToken,
    #[error("session token signature is invalid")]
    BadSignature,
    #[error("session token is expired")]
    Expired,
    #[error("session token is malformed")]
    Malformed,
    #[error("session signing key is invalid")]
    InvalidKey,
}
