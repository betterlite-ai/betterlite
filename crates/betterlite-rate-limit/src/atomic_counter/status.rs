use crate::limiter_records::{AtomicCounterMeta, RateLimitStatus};

use super::rate_limit_kind::status_rate_limit_type;

#[must_use]
pub fn counter_status(
    code: &str,
    current_limit: i64,
    counted: i64,
    meta: &AtomicCounterMeta,
) -> RateLimitStatus {
    RateLimitStatus {
        code: code.to_owned(),
        current_limit,
        limit_remaining: current_limit.saturating_sub(counted).max(0),
        rate_limit_type: status_rate_limit_type(meta.rate_limit_type),
        descriptor_key: meta.descriptor_key.clone(),
        descriptor_value: Some(meta.descriptor_value.clone()),
    }
}
