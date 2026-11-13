use betterlite_error::rate_limit::RateLimitCategory;
use thiserror::Error;

use super::clock::ROUTING_ARGS_TTL_SECONDS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoChannel {
    Itpm,
    Otpm,
}

impl IoChannel {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Itpm => "ITPM",
            Self::Otpm => "OTPM",
        }
    }
}

// every field litellm.RateLimitError receives or sets in _rate_limit_error
#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[error("the {channel:?} token reservation of {limit} is over at {current}")]
pub struct RateLimitExceeded {
    pub channel: IoChannel,
    pub limit: i64,
    pub current: i64,
}

impl RateLimitExceeded {
    pub const STATUS_CODE: u16 = 429;
    pub const CODE: &'static str = "429";
    pub const ERROR_TYPE: &'static str = "throttling_error";
    pub const CATEGORY: RateLimitCategory = RateLimitCategory::VendorRateLimit;
    pub const RETRY_AFTER_SECONDS: u64 = ROUTING_ARGS_TTL_SECONDS;
    pub const NUM_RETRIES: u32 = 0;
    pub const LLM_PROVIDER: &'static str = "";
    pub const MODEL: &'static str = "";
}

#[derive(Debug, Error)]
pub enum IoPreCallError {
    #[error(transparent)]
    RateLimit(#[from] RateLimitExceeded),
    #[error("{field} must be an integer")]
    OutputCap { field: &'static str },
}
