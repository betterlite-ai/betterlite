use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ObservationError {
    #[error("observation version must be 3")]
    Version,
    #[error("request_id must not be empty")]
    EmptyRequestId,
    #[error("started_at must be finite and nonnegative")]
    InvalidStartedAt,
    #[error("available_at must be finite and nonnegative")]
    InvalidAvailableAt,
    #[error("minimum_cache_tokens must not be negative")]
    NegativeMinimumCacheTokens,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum BaselineHistoryError {
    #[error("a token count computation left the representable 64-bit range")]
    CounterOverflow,
    #[error("a cache timestamp computation left the finite representable range")]
    TimestampOutOfRange,
    #[error("a cache TTL outside the valid 300/3600 second domain reached checked arithmetic")]
    InvalidTtl,
}
