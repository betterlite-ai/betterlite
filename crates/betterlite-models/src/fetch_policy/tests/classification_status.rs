use rstest::rstest;

use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, classify_fetch_response};

#[derive(Debug, PartialEq, Eq)]
enum Expected {
    Reloaded,
    Unavailable,
    Retryable,
}

#[rstest]
#[case::below_client_errors(399, Expected::Reloaded)]
#[case::client_error(400, Expected::Unavailable)]
#[case::not_found(404, Expected::Unavailable)]
#[case::not_implemented(501, Expected::Unavailable)]
#[case::last_server_error(599, Expected::Unavailable)]
#[case::above_server_errors(600, Expected::Reloaded)]
#[case::rate_limit(429, Expected::Retryable)]
#[case::internal(500, Expected::Retryable)]
#[case::bad_gateway(502, Expected::Retryable)]
#[case::service_unavailable(503, Expected::Retryable)]
#[case::gateway_timeout(504, Expected::Retryable)]
fn catalog_fetch_policy_behavior_tests_behavior(#[case] status: u16, #[case] expected: Expected) {
    match classify_fetch_response(support::response(status, "{}"), support::URL) {
        AttemptOutcome::Reloaded(fetched) => {
            assert_eq!(expected, Expected::Reloaded);
            assert!(fetched.models.is_empty());
        }
        AttemptOutcome::Unavailable(unavailable) => {
            assert_eq!(expected, Expected::Unavailable);
            assert_eq!(
                unavailable.reason,
                format!("HTTP {status} from {}", support::URL)
            );
        }
        AttemptOutcome::Retryable(retryable) => {
            assert_eq!(expected, Expected::Retryable);
            assert_eq!(
                retryable.reason,
                format!("HTTP {status} from {}", support::URL)
            );
            assert_eq!(retryable.retry_after_seconds, None);
        }
    }
}
