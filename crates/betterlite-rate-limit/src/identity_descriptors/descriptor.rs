use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

// Every identity bucket carries the handler's window and never a parallel-request ceiling
#[must_use]
pub fn rate_limit_descriptor(
    key: &str,
    value: String,
    requests_per_unit: Option<i64>,
    tokens_per_unit: Option<i64>,
    window_size: i64,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value,
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit,
            tokens_per_unit,
            max_parallel_requests: None,
            window_size: Some(window_size),
        }),
    }
}
