use crate::rate_limiters::legacy_rate_limit_counters::error::LegacyRateLimitError;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::UsageCacheError;

// what the pre-call hook refuses with: B3047's 429, or the usage cache breaking its own contract
#[derive(Debug, thiserror::Error)]
pub enum LegacyAdmissionError {
    #[error(transparent)]
    RateLimit(#[from] LegacyRateLimitError),
    #[error(transparent)]
    Cache(#[from] UsageCacheError),
}
