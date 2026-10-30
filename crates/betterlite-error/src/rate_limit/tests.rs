#![cfg(test)]

use rstest::rstest;

use super::{RateLimitCategory, RateLimitDimension};

#[rstest]
#[case::vendor(RateLimitCategory::VendorRateLimit, "vendor_rate_limit")]
#[case::vendor_batch(RateLimitCategory::VendorBatchRateLimit, "vendor_batch_rate_limit")]
#[case::litellm(RateLimitCategory::LitellmRateLimit, "litellm_rate_limit")]
#[case::litellm_batch(RateLimitCategory::LitellmBatchRateLimit, "litellm_batch_rate_limit")]
fn rate_limit_category_tests_behavior(#[case] category: RateLimitCategory, #[case] wire: &str) {
    assert_eq!(serde_json::to_value(category).unwrap(), wire);
}

#[rstest]
#[case::requests(RateLimitDimension::Requests, "requests")]
#[case::tokens(RateLimitDimension::Tokens, "tokens")]
#[case::concurrent(RateLimitDimension::ConcurrentRequests, "concurrent_requests")]
#[case::budget(RateLimitDimension::Budget, "budget")]
#[case::max_iterations(RateLimitDimension::MaxIterations, "max_iterations")]
fn rate_limit_dimension_tests_behavior(#[case] dimension: RateLimitDimension, #[case] wire: &str) {
    assert_eq!(serde_json::to_value(dimension).unwrap(), wire);
}
