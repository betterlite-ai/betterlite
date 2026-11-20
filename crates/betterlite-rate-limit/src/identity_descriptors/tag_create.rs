use betterlite_core::api::request_destination::RequestData;

use super::tag_descriptor::tag_rate_limit_descriptor;
use super::tag_fromkeys::deduplicated_tags;
use crate::limiter_records::{RateLimitDescriptor, TagRateLimitResolver};

mod tests;

// LiteLLM's _create_tag_rate_limit_descriptors: one tag descriptor per deduplicated request tag the resolver knows
pub async fn create_tag_descriptors(
    data: &RequestData,
    window_size: i64,
    resolver: &dyn TagRateLimitResolver,
) -> Vec<RateLimitDescriptor> {
    let tags = deduplicated_tags(data);
    if tags.is_empty() {
        return Vec::new();
    }
    let tag_limits = resolver.resolve(&tags).await;
    let mut descriptors = Vec::new();
    for tag in tags {
        let Some(limit) = tag_limits.get(&tag) else {
            continue;
        };
        descriptors.push(tag_rate_limit_descriptor(&tag, *limit, window_size));
    }
    descriptors
}
