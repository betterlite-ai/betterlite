#![cfg(test)]

use super::Class;
use crate::error::ErrorClass;
use rstest::rstest;

#[rstest]
#[case::authentication(
    r#"{"family":"authentication","message":"m"}"#,
    Some("401_AUTHENTICATION_FAILED")
)]
#[case::not_found(
    r#"{"family":"not_found","message":"m"}"#,
    Some("404_RESOURCE_NOT_FOUND")
)]
#[case::permission(
    r#"{"family":"permission_denied","message":"m"}"#,
    Some("403_PERMISSION_DENIED")
)]
#[case::request(
    r#"{"family":"bad_request","message":"m"}"#,
    Some("400_INVALID_REQUEST")
)]
#[case::unprocessable(
    r#"{"family":"unprocessable_entity","message":"m"}"#,
    Some("400_INVALID_REQUEST")
)]
#[case::context(
    r#"{"family":"context_window_exceeded","message":"m"}"#,
    Some("400_CONTEXT_WINDOW_EXCEEDED")
)]
#[case::policy(
    r#"{"family":"content_policy_violation","message":"m"}"#,
    Some("400_CONTENT_POLICY_VIOLATION")
)]
#[case::unavailable(
    r#"{"family":"service_unavailable","message":"m"}"#,
    Some("503_PROVIDER_OVERLOADED")
)]
#[case::gateway(
    r#"{"family":"bad_gateway","message":"m"}"#,
    Some("500_PROVIDER_INTERNAL_ERROR")
)]
#[case::internal(
    r#"{"family":"internal_server","message":"m"}"#,
    Some("500_PROVIDER_INTERNAL_ERROR")
)]
#[case::connection(
    r#"{"family":"api_connection","message":"m"}"#,
    Some("500_PROVIDER_CONNECTION_ERROR")
)]
#[case::unsupported(
    r#"{"family":"unsupported_params","message":"m"}"#,
    Some("400_INVALID_PARAMETER_VALUE")
)]
#[case::timeout(
    r#"{"family":"timeout","context":{"message":"m"}}"#,
    Some("408_UPSTREAM_TIMEOUT")
)]
#[case::rate_limit(
    r#"{"family":"rate_limit","context":{"message":"m"},"category":"vendor_rate_limit"}"#,
    Some("429_RATE_LIMIT_EXCEEDED")
)]
#[case::budget(
    r#"{"family":"budget_exceeded","current_cost":2,"max_budget":1,"status_code":422}"#,
    Some("429_BUDGET_EXCEEDED")
)]
#[case::guardrail(
    r#"{"family":"guardrail_raised","message":"m","status_code":400,"blocked_content":true}"#,
    Some("400_CONTENT_POLICY_VIOLATION")
)]
#[case::fallback(r#"{"family":"mid_stream_fallback","context":{"message":"m"},"generated_content":"","is_pre_first_chunk":true}"#, Some("500_ROUTER_FALLBACK_FAILURE"))]
#[case::api(r#"{"family":"api","context":{"message":"m"},"status":500}"#, None)]
#[case::response_validation(r#"{"family":"api_response_validation","message":"m"}"#, None)]
#[case::schema(r#"{"family":"json_schema_validation","model":"m","provider":"p","raw_response":"","schema":""}"#, None)]
#[case::unknown(r#"{"family":"unclassified","message":"m"}"#, None)]
fn error_normalization_tests_behavior(#[case] json: &str, #[case] expected: Option<&str>) {
    let class = serde_json::from_str::<ErrorClass>(json).unwrap();
    assert_eq!(
        Class::from(&class).code(class.metadata().rate_limit_dimension),
        expected
    );
}
