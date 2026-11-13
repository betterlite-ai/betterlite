use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, classify_fetch_response};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let response = support::response_with_headers(429, &[("retry-after", "2.5")], "{}");
    match classify_fetch_response(response, support::URL) {
        AttemptOutcome::Retryable(retryable) => {
            assert_eq!(retryable.retry_after_seconds, Some(support::decimal("2.5")));
        }
        AttemptOutcome::Reloaded(_) | AttemptOutcome::Unavailable(_) => {
            panic!("a retryable status carries its Retry-After");
        }
    }
}
