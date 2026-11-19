use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;

use crate::request_stash::RequestStashSlot;
use crate::usage_resolution::Usage;

pub struct FailureEvent<'request> {
    pub stash: &'request RequestStashSlot,
    pub call_id: Option<&'request str>,
    pub combined_usage: Option<&'request Usage>,
    pub rate_limit_type: TokenRateLimitType,
    pub window_size: i64,
}
