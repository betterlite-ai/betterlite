#![cfg(test)]

use rstest::rstest;

use super::*;

#[rstest]
#[case::authorization_denied_over_status(
    "authorization denied for this api key",
    500,
    "authentication"
)]
#[case::context_window_over_status(
    "this model's maximum context length is 4096 tokens",
    500,
    "context_window_exceeded"
)]
#[case::token_quota_over_status("token_quota_reached for this account", 500, "rate_limit")]
#[case::upstream_invalid_response_over_status(
    "The server received an invalid response from an upstream server.",
    429,
    "internal_server"
)]
#[case::watsonx_route_over_status(
    "model_no_support_for_function for this deployment",
    429,
    "bad_request"
)]
fn openai_like_errors_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] conflicting_status: u16,
    #[case] expected_family: &str,
) {
    let error = facts(Some(conflicting_status));
    let mapped = map_openai_like_exception(&error, error_str, "my_provider", None)
        .expect("a string predicate always produces a class");
    let value = serde_json::to_value(&mapped).expect("ErrorClass always serializes");
    assert_eq!(value["family"], serde_json::json!(expected_family));
}
