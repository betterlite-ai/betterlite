#![cfg(test)]

use super::Class;
use crate::error::ErrorClass;
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::authentication("authentication", Some("401_AUTHENTICATION_FAILED"))]
#[case::not_found("not_found", Some("404_RESOURCE_NOT_FOUND"))]
#[case::permission("permission", Some("403_PERMISSION_DENIED"))]
#[case::request("request", Some("400_INVALID_REQUEST"))]
#[case::unprocessable("unprocessable", Some("400_INVALID_REQUEST"))]
#[case::context("context", Some("400_CONTEXT_WINDOW_EXCEEDED"))]
#[case::policy("policy", Some("400_CONTENT_POLICY_VIOLATION"))]
#[case::unavailable("unavailable", Some("503_PROVIDER_OVERLOADED"))]
#[case::gateway("gateway", Some("500_PROVIDER_INTERNAL_ERROR"))]
#[case::internal("internal", Some("500_PROVIDER_INTERNAL_ERROR"))]
#[case::connection("connection", Some("500_PROVIDER_CONNECTION_ERROR"))]
#[case::unsupported("unsupported", Some("400_INVALID_PARAMETER_VALUE"))]
#[case::timeout("timeout", Some("408_UPSTREAM_TIMEOUT"))]
#[case::rate_limit("rate_limit", Some("429_RATE_LIMIT_EXCEEDED"))]
#[case::budget("budget", Some("429_BUDGET_EXCEEDED"))]
#[case::guardrail("guardrail", Some("400_CONTENT_POLICY_VIOLATION"))]
#[case::fallback("fallback", Some("500_ROUTER_FALLBACK_FAILURE"))]
#[case::api("api", None)]
#[case::response_validation("response_validation", None)]
#[case::schema("schema", None)]
#[case::unknown("unknown", None)]
fn error_normalization_tests_behavior(#[case] name: &str, #[case] expected: Option<&str>) {
    let mut classes: BTreeMap<String, ErrorClass> =
        serde_json::from_str(include_str!("fixtures/classes.json")).unwrap();
    let class = classes.remove(name).unwrap();
    let actual = Class::from(&class).code(class.metadata().rate_limit_dimension);
    assert_eq!(actual, expected);
}
