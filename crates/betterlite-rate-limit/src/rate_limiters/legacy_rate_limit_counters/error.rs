use betterlite_error::rate_limit::{InvalidRateLimitHeader, ProxyRateLimitError};

// what a refused legacy check answers: the 429 to forward, or the header it could not build
#[derive(Debug, thiserror::Error)]
pub enum LegacyRateLimitError {
    #[error("{0}")]
    Rejected(Box<ProxyRateLimitError>),
    #[error(transparent)]
    Header(#[from] InvalidRateLimitHeader),
}
