#![cfg(test)]

use rstest::rstest;

use super::*;
use crate::error::ErrorClass;
use crate::rate_limit::RateLimitCategory;

#[rstest]
#[case::token_quota_predicate("token_quota_reached for this account", None)]
#[case::status_rate_limited("failed", Some(429))]
fn openai_like_errors_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
) {
    let error = facts(status_code);
    let mapped = map_openai_like_exception(&error, error_str, "my_provider", None)
        .expect("a rate-limited call always produces a class");
    let ErrorClass::RateLimit(details) = mapped else {
        panic!("expected the rate_limit family");
    };
    assert_eq!(details.category, RateLimitCategory::VendorRateLimit);
    assert_eq!(details.dimension, None);
    assert!(details.headers.is_empty());
}
