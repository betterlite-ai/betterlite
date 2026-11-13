use betterlite_context::local_ttl_cache::CacheError;
use thiserror::Error;

pub type WindowResult<T> = Result<T, IncrementError>;

#[derive(Debug, Error)]
pub enum IncrementError {
    #[error("the local budget window cache failed")]
    Cache {
        #[from]
        source: CacheError,
    },
    #[error("the budget window time of {seconds} seconds is not a finite non-negative number")]
    InvalidSeconds { seconds: f64 },
    #[error("the cached budget window value under {key} is not the expected type")]
    InvalidCacheValue { key: String },
    #[error("the spend under {key} overflowed a decimal")]
    SpendOverflow { key: String },
    #[error("a budget window ttl of {seconds} seconds does not fit redis")]
    TtlOutOfRange { seconds: u64 },
}
