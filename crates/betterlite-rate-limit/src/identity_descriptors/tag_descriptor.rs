use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit, TagRateLimit};

mod tests;

// LiteLLM's _tag_rate_limit_descriptor: the tag's own bucket carries both ceilings and the handler's window
#[must_use]
pub fn tag_rate_limit_descriptor(
    tag: &str,
    limit: TagRateLimit,
    window_size: i64,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "tag".to_owned(),
        value: tag.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: limit.rpm_limit,
            tokens_per_unit: limit.tpm_limit,
            max_parallel_requests: None,
            window_size: Some(window_size),
        }),
    }
}
