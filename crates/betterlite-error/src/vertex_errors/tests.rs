#![cfg(test)]

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
// branch-1153: the two project-resolution phrases
#[case::project_not_used(
    "Vertex AI API has not been used in project 1234 before or it is disabled",
    None,
    Some("bad_request"),
    Some(400)
)]
#[case::project_not_found("Unable to find your project", None, Some("bad_request"), Some(400))]
// branch-1167: the payload-size phrase, independent of status_code
#[case::payload_too_large(
    "400 Request payload size exceeds the maximum limit of 20971520 bytes",
    Some(400),
    Some("context_window_exceeded"),
    Some(400)
)]
// branch-1173: the shared context-window checker
#[case::context_window_phrase(
    "This model's maximum context length is 32760 tokens, however you requested 40000 tokens",
    None,
    Some("context_window_exceeded"),
    Some(400)
)]
// branch-1180: the two empty-response phrases
#[case::none_unknown_error(
    "None Unknown Error. please retry the request",
    None,
    Some("internal_server"),
    Some(500)
)]
#[case::content_has_no_parts("Content has no parts.", None, Some("internal_server"), Some(500))]
// branch-1192
#[case::invalid_api_key(
    "API key not valid. Please pass a valid API key.",
    None,
    Some("authentication"),
    Some(401)
)]
// branch-1199: the "403" substring wins over any status_code fallback
#[case::forbidden_string(
    "403 PERMISSION_DENIED: the caller does not have permission",
    None,
    Some("bad_request"),
    Some(400)
)]
// branch-1213: the two content-blocked phrases
#[case::response_blocked(
    "The response was blocked.",
    None,
    Some("content_policy_violation"),
    Some(400)
)]
#[case::content_filtered(
    "Output blocked by content filtering policy",
    None,
    Some("content_policy_violation"),
    Some(400)
)]
// branch-1230: all five quota phrases
#[case::quota_exceeded_429(
    "429 Quota exceeded for aiplatform.googleapis.com/generate_content_requests_per_minute",
    None,
    Some("rate_limit"),
    Some(429)
)]
#[case::quota_exceeded_for(
    "Quota exceeded for generate_content_requests",
    None,
    Some("rate_limit"),
    Some(429)
)]
#[case::resource_exhausted("Resource exhausted", None, Some("rate_limit"), Some(429))]
#[case::index_error(
    "IndexError: list index out of range",
    None,
    Some("rate_limit"),
    Some(429)
)]
#[case::temporarily_out_of_capacity(
    "429 Unable to submit request because the service is temporarily out of capacity.",
    None,
    Some("rate_limit"),
    Some(429)
)]
// branch-1250: the 5xx-wrapped-429 envelope, as a JSON number and as a JSON string, plus its bounds
#[case::envelope_429_int(
    r#"{"error":{"code":429,"message":"wrapped"}}"#,
    Some(503),
    Some("rate_limit"),
    Some(429)
)]
#[case::envelope_429_string(
    r#"{"error":{"code":"429","message":"wrapped"}}"#,
    Some(500),
    Some("rate_limit"),
    Some(429)
)]
#[case::envelope_boundary_included(
    r#"{"error":{"code":429}}"#,
    Some(500),
    Some("rate_limit"),
    Some(429)
)]
#[case::envelope_boundary_excluded(r#"{"error":{"code":429}}"#, Some(600), None, None)]
#[case::envelope_wrong_code(r#"{"error":{"code":500}}"#, Some(504), None, None)]
#[case::envelope_malformed_json("not json at all", Some(501), None, None)]
#[case::envelope_missing_error_key(r#"{"foo":"bar"}"#, Some(501), None, None)]
#[case::envelope_error_not_object(r#"{"error":"boom"}"#, Some(501), None, None)]
#[case::envelope_code_not_integer(r#"{"error":{"code":"unreachable"}}"#, Some(501), None, None)]
// branch-1272: the two internal-server phrases
#[case::internal_server_literal(
    "500 Internal Server Error",
    None,
    Some("internal_server"),
    Some(500)
)]
#[case::model_overloaded("The model is overloaded.", None, Some("internal_server"), Some(500))]
// branch-1279/1280..1358: the status_code fallback, one case per recognized status, plus unmatched statuses
#[case::status_bad_request("an unrelated failure", Some(400), Some("bad_request"), Some(400))]
#[case::status_authentication("an unrelated failure", Some(401), Some("authentication"), Some(401))]
#[case::status_permission_denied(
    "the caller lacks permission for this resource",
    Some(403),
    Some("permission_denied"),
    Some(403)
)]
#[case::status_not_found("an unrelated failure", Some(404), Some("not_found"), Some(404))]
#[case::status_timeout("an unrelated failure", Some(408), Some("timeout"), Some(408))]
#[case::status_rate_limit("an unrelated failure", Some(429), Some("rate_limit"), Some(429))]
#[case::status_internal_server(
    "an unrelated failure",
    Some(500),
    Some("internal_server"),
    Some(500)
)]
#[case::status_api_connection("an unrelated failure", Some(502), Some("api_connection"), Some(500))]
#[case::status_service_unavailable(
    "an unrelated failure",
    Some(503),
    Some("service_unavailable"),
    Some(503)
)]
#[case::status_unmatched_in_range("an unrelated failure", Some(418), None, None)]
#[case::status_unmatched_504("an unrelated failure", Some(504), None, None)]
// unmatched inputs with no status_code at all
#[case::no_status_no_match("an unrelated failure", None, None, None)]
#[case::empty_error_str_no_status("", None, None, None)]
#[case::empty_error_str_with_status("", Some(401), Some("authentication"), Some(401))]
// the text predicates win over a status_code that would otherwise map to a different family
#[case::precedence_text_over_status(
    "API key not valid. despite a 500 status",
    Some(500),
    Some("authentication"),
    Some(401)
)]
fn vertex_errors_behavior_tests_behavior(
    #[case] error_str: &str,
    #[case] status_code: Option<u16>,
    #[case] expected_family: Option<&str>,
    #[case] expected_wire_status: Option<u16>,
) {
    let error = facts(status_code);
    let mapped = map_vertex_exception(
        &error,
        error_str,
        "vertex_ai",
        Some("gemini-1.5-pro".to_owned()),
    );
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
