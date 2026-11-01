#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(
    error_str: &str,
    exception_type: &str,
    status_code: Option<u16>,
) -> ReplicateExceptionInput {
    ReplicateExceptionInput {
        model: "replicate/some-model".to_owned(),
        error_str: error_str.to_owned(),
        exception_type: exception_type.to_owned(),
        status_code,
    }
}

#[rstest]
#[case::incorrect_authentication_token(
    "Incorrect authentication token provided",
    "APIError",
    None,
    "authentication",
    None
)]
#[case::input_too_long(
    "input is too long for this model",
    "APIError",
    None,
    "context_window_exceeded",
    None
)]
#[case::model_error("generic failure", "ModelError", None, "bad_request", None)]
#[case::request_throttled(
    "Request was throttled by replicate",
    "APIError",
    None,
    "rate_limit",
    None
)]
#[case::hasattr_status_unmatched("generic failure", "APIError", Some(599), "api", Some(500))]
#[case::status_401("generic failure", "APIError", Some(401), "authentication", Some(401))]
#[case::status_413("generic failure", "APIError", Some(413), "bad_request", Some(400))]
#[case::status_422(
    "generic failure",
    "APIError",
    Some(422),
    "unprocessable_entity",
    Some(422)
)]
#[case::status_408("generic failure", "APIError", Some(408), "timeout", Some(408))]
#[case::status_429("generic failure", "APIError", Some(429), "rate_limit", Some(429))]
#[case::status_500(
    "generic failure",
    "APIError",
    Some(500),
    "service_unavailable",
    Some(503)
)]
#[case::generic_fallback("totally unrelated failure text", "APIError", None, "api", Some(500))]
fn anthropic_replicate_errors_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] exception_type: &str,
    #[case] status_code: Option<u16>,
    #[case] expected_family: &str,
    #[case] expected_wire_status: Option<u16>,
) {
    let class = map_replicate_exception(&input(error_str, exception_type, status_code));
    let value = serde_json::to_value(&class).expect("ErrorClass always serializes");
    assert_eq!(value["family"], serde_json::json!(expected_family));
    if let Some(status) = expected_wire_status {
        assert_eq!(class.metadata().status_code, status);
    }
}

#[test]
fn text_predicate_wins_over_a_competing_status_code() {
    let class = map_replicate_exception(&input(
        "Request was throttled by replicate",
        "APIError",
        Some(401),
    ));
    let value = serde_json::to_value(&class).expect("ErrorClass always serializes");
    assert_eq!(value["family"], serde_json::json!("rate_limit"));
}
