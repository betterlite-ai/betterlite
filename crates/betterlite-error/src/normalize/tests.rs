#![cfg(test)]

use rstest::rstest;

use super::{ErrorFacts, normalize_error};

struct Case {
    facts: ErrorFacts,
    expected: Option<&'static str>,
}

#[rstest]
#[case::proxy_type_first(Case { facts: ErrorFacts { proxy_type: Some("budget_exceeded".to_owned()), message: "ordinary".to_owned(), class: Some("Timeout".to_owned()), status_code: Some(408), rate_limit_type: None }, expected: Some("429_BUDGET_EXCEEDED") })]
#[case::message_before_class(Case { facts: ErrorFacts { proxy_type: None, message: "the request timed out".to_owned(), class: Some("BadRequestError".to_owned()), status_code: Some(400), rate_limit_type: None }, expected: Some("408_UPSTREAM_TIMEOUT") })]
#[case::budget_rate_limit(Case { facts: ErrorFacts { proxy_type: None, message: "rate limited".to_owned(), class: Some("RateLimitError".to_owned()), status_code: Some(429), rate_limit_type: Some("budget".to_owned()) }, expected: Some("429_BUDGET_EXCEEDED") })]
#[case::vendor_rate_limit(Case { facts: ErrorFacts { proxy_type: None, message: "rate limited".to_owned(), class: Some("RateLimitError".to_owned()), status_code: Some(429), rate_limit_type: Some("requests".to_owned()) }, expected: Some("429_RATE_LIMIT_EXCEEDED") })]
#[case::class_before_router_wrapper(Case { facts: ErrorFacts { proxy_type: None, message: "no fallback model group found".to_owned(), class: Some("AuthenticationError".to_owned()), status_code: Some(500), rate_limit_type: None }, expected: Some("401_AUTHENTICATION_FAILED") })]
#[case::status_fallback(Case { facts: ErrorFacts { proxy_type: None, message: "other".to_owned(), class: None, status_code: Some(502), rate_limit_type: None }, expected: Some("500_PROVIDER_INTERNAL_ERROR") })]
#[case::unclassified(Case { facts: ErrorFacts { proxy_type: None, message: "other".to_owned(), class: None, status_code: Some(418), rate_limit_type: None }, expected: Some("UNCLASSIFIED") })]
#[case::absent_exception(Case { facts: ErrorFacts { proxy_type: None, message: "other".to_owned(), class: None, status_code: None, rate_limit_type: None }, expected: None })]
fn error_normalization_tests_behavior(#[case] case: Case) {
    assert_eq!(normalize_error(&case.facts), case.expected);
}
