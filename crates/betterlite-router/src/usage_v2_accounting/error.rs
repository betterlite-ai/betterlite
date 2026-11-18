use betterlite_error::rate_limit::RateLimitCategory;
use rust_decimal::Decimal;
use thiserror::Error;

// every field litellm.RateLimitError receives for a deployment over its own rpm limit
#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[error("deployment {model} is over its rpm limit={limit}; current usage={usage}")]
pub struct RpmLimitExceeded {
    pub limit: i64,
    pub usage: Decimal,
    pub model: String,
    pub num_retries: Option<i64>,
    pub retry_after_seconds: Option<u64>,
}

impl RpmLimitExceeded {
    pub const STATUS_CODE: u16 = 429;
    pub const CODE: &'static str = "429";
    pub const ERROR_TYPE: &'static str = "throttling_error";
    pub const CATEGORY: RateLimitCategory = RateLimitCategory::VendorRateLimit;
    // async_pre_call_check passes headers={"retry-after": str(60)}; the sync check passes none
    pub const RETRY_AFTER_SECONDS: u64 = 60;
}

#[derive(Debug, Error)]
pub enum UsageV2Error {
    #[error(transparent)]
    RateLimit(#[from] RpmLimitExceeded),
}
