use betterlite_auth::auth_limit_inputs::error::AuthLimitInputsError;

use crate::rate_limiters::legacy_rate_limit_counters::error::LegacyRateLimitError;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::UsageCacheError;

// what the pre-call hook refuses with: B3047's 429, the usage cache breaking its contract, or B745's lookup
#[derive(Debug, thiserror::Error)]
pub enum LegacyAdmissionError {
    #[error(transparent)]
    RateLimit(#[from] LegacyRateLimitError),
    #[error(transparent)]
    Cache(#[from] UsageCacheError),
    #[error(transparent)]
    LimitInputs(#[from] AuthLimitInputsError),
}
