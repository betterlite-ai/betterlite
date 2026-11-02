#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::*;
use crate::facts::ProviderErrorFacts;

fn facts(status_code: Option<u16>) -> ProviderErrorFacts {
    ProviderErrorFacts {
        status_code,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    }
}

#[rstest]
#[case::authorization_denied(
    "authorization denied for this api key",
    None,
    Some("authentication"),
    Some(401)
)]
#[case::context_window(
    "this model's maximum context length is 4096 tokens",
    None,
    Some("context_window_exceeded"),
    Some(400)
)]
#[case::token_quota_reached(
    "token_quota_reached for this account",
    None,
    Some("rate_limit"),
    Some(429)
)]
#[case::upstream_invalid_response(
    "The server received an invalid response from an upstream server.",
    None,
    Some("internal_server"),
    Some(500)
)]
#[case::watsonx_route(
    "model_no_support_for_function for this deployment",
    None,
    Some("bad_request"),
    Some(400)
)]
#[case::status_internal_server("failed", Some(500), Some("internal_server"), Some(500))]
#[case::status_authentication("failed", Some(401), Some("authentication"), Some(401))]
#[case::status_permission_denied("failed", Some(403), Some("permission_denied"), Some(403))]
#[case::status_bad_request("failed", Some(400), Some("bad_request"), Some(400))]
#[case::status_not_found("failed", Some(404), Some("not_found"), Some(404))]
#[case::status_timeout_408("failed", Some(408), Some("timeout"), Some(408))]
#[case::status_unprocessable("failed", Some(422), Some("bad_request"), Some(400))]
#[case::status_failed_dependency("failed", Some(424), Some("bad_request"), Some(400))]
#[case::status_rate_limit("failed", Some(429), Some("rate_limit"), Some(429))]
#[case::status_service_unavailable("failed", Some(503), Some("service_unavailable"), Some(503))]
#[case::status_gateway_timeout("failed", Some(504), Some("timeout"), Some(504))]
#[case::status_no_match("failed", Some(418), None, None)]
#[case::no_predicate_no_status("failed", None, None, None)]
fn openai_like_errors_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
    #[case] expected_family: Option<&str>,
    #[case] expected_wire_status: Option<u16>,
) {
    let error = facts(status_code);
    let mapped =
        map_openai_like_exception(&error, error_str, "my_provider", Some("gpt-4".to_owned()));
    match expected_family {
        None => assert!(mapped.is_none()),
        Some(family) => {
            let class = mapped.expect("a matching predicate must produce a class");
            let value = serde_json::to_value(&class).expect("ErrorClass always serializes");
            assert_eq!(value["family"], serde_json::json!(family));
            assert_eq!(
                class.metadata().status_code,
                expected_wire_status.expect("a matched case always carries an expected status")
            );
        }
    }
}

fn facts_with_response(status_code: Option<u16>) -> ProviderErrorFacts {
    use crate::facts::{RequestFacts, ResponseFacts};
    let mut error = facts(status_code);
    error.response = status_code.map(|status| ResponseFacts {
        status_code: status,
        headers: BTreeMap::new(),
        request: Some(RequestFacts {
            method: "POST".to_owned(),
            url: "https://my-provider.example/v1".to_owned(),
        }),
    });
    error
}

#[rstest]
#[case::context_window_on_500("this model's maximum context length is 4096 tokens", 500)]
fn response_branches_answer_the_provider_status(#[case] error_str: &str, #[case] status: u16) {
    let error = facts_with_response(Some(status));
    let class = map_openai_like_exception(&error, error_str, "my_provider", None)
        .expect("expected a classification");
    assert_eq!(class.metadata().status_code, status);
}

#[rstest]
#[case::authorization_denied_without_a_response("authorization denied for this api key", None, 401)]
#[case::status_unprocessable("failed", Some(422), 400)]
#[case::status_failed_dependency("failed", Some(424), 400)]
#[case::status_bad_request("failed", Some(400), 400)]
#[case::status_authentication("failed", Some(401), 401)]
#[case::status_not_found("failed", Some(404), 404)]
fn branches_without_a_response_keep_the_class_status(
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
    #[case] expected_status: u16,
) {
    let error = facts_with_response(status_code);
    let class = map_openai_like_exception(&error, error_str, "my_provider", None)
        .expect("expected a classification");
    assert_eq!(class.metadata().status_code, expected_status);
}

#[path = "tests/precedence/tests.rs"]
mod precedence;

#[path = "tests/rate_limit_defaults/tests.rs"]
mod rate_limit_defaults;

#[path = "tests/context_passthrough/tests.rs"]
mod context_passthrough;
