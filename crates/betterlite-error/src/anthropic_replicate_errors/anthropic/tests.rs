#![cfg(test)]

use rstest::rstest;

use super::*;

fn input(error_str: &str, status_code: Option<u16>) -> AnthropicExceptionInput {
    AnthropicExceptionInput {
        model: "claude-3-opus".to_owned(),
        error_str: error_str.to_owned(),
        status_code,
    }
}

#[rstest]
#[case::context_window_prompt_too_long(
    "prompt is too long: 205000 tokens > 200000 maximum",
    None,
    Some("context_window_exceeded"),
    None
)]
#[case::context_window_prompt_length(
    "prompt: length exceeds the maximum",
    None,
    Some("context_window_exceeded"),
    None
)]
#[case::context_window_checker(
    "this model's maximum context length is 200000 tokens",
    None,
    Some("context_window_exceeded"),
    None
)]
#[case::overloaded_error(
    "overloaded_error: the service is overloaded",
    None,
    Some("internal_server"),
    None
)]
#[case::overloaded_capitalized("Overloaded", None, Some("internal_server"), None)]
#[case::invalid_api_key("Invalid API Key provided", None, Some("authentication"), None)]
#[case::content_filtering_policy(
    "blocked by the content filtering policy",
    None,
    Some("content_policy_violation"),
    None
)]
#[case::client_error_400(
    "Client error '400 Bad Request' for url https://api.anthropic.com",
    None,
    Some("bad_request"),
    None
)]
#[case::hasattr_status_unmatched("generic failure", Some(599), None, None)]
#[case::status_401("generic failure", Some(401), Some("authentication"), Some(401))]
#[case::status_403("generic failure", Some(403), Some("permission_denied"), Some(403))]
#[case::status_413("generic failure", Some(413), Some("bad_request"), Some(400))]
#[case::status_404("generic failure", Some(404), Some("not_found"), Some(404))]
#[case::status_408("generic failure", Some(408), Some("timeout"), Some(408))]
#[case::status_429("generic failure", Some(429), Some("rate_limit"), Some(429))]
#[case::status_529("generic failure", Some(529), Some("internal_server"), Some(500))]
#[case::status_502("generic failure", Some(502), Some("bad_gateway"), Some(502))]
#[case::status_503("generic failure", Some(503), Some("service_unavailable"), Some(503))]
#[case::status_504("generic failure", Some(504), Some("timeout"), Some(504))]
#[case::no_predicate_no_status("generic failure", None, None, None)]
fn anthropic_replicate_errors_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
    #[case] expected_family: Option<&str>,
    #[case] expected_wire_status: Option<u16>,
) {
    let mapped = map_anthropic_exception(&input(error_str, status_code));
    match expected_family {
        None => assert!(mapped.is_none()),
        Some(family) => {
            let class = mapped.expect("a matching predicate must produce a class");
            let value = serde_json::to_value(&class).expect("ErrorClass always serializes");
            assert_eq!(value["family"], serde_json::json!(family));
            if let Some(status) = expected_wire_status {
                assert_eq!(class.metadata().status_code, status);
            }
        }
    }
}

#[test]
fn text_predicate_wins_over_a_competing_status_code() {
    let mapped =
        map_anthropic_exception(&input("blocked by the content filtering policy", Some(429)));
    let class = mapped.expect("the content policy predicate must match before status_code is read");
    let value = serde_json::to_value(&class).expect("ErrorClass always serializes");
    assert_eq!(
        value["family"],
        serde_json::json!("content_policy_violation")
    );
}
