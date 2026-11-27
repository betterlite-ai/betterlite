use betterlite_cache::backend::BackendError;
use thiserror::Error;

use super::redis::ThrottleRedisError;

#[derive(Debug, Error)]
pub enum LoginThrottleError {
    #[error(transparent)]
    Local(#[from] BackendError),
    #[error(transparent)]
    Shared(#[from] ThrottleRedisError),
    #[error("the cached login block expiry under {key} is not a finite number")]
    InvalidBlockExpiry { key: String },
    #[error("the login counter under {key} is not a whole number")]
    InvalidCount { key: String },
    #[error("the login throttle ttl of {seconds} seconds cannot be represented as a float")]
    UnrepresentableTtl { seconds: i64 },
}
