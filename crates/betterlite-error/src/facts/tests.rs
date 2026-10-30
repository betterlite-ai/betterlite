#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::*;

#[rstest]
#[case::phrase("rate limit reached", None, true)]
#[case::number_with_429("429", Some(429), true)]
#[case::number_with_other_status("429", Some(400), false)]
#[case::service_tier("service tier capacity exceeded", Some(400), true)]
#[case::hyphenated("rate-limit reached", Some(400), true)]
#[case::ordinary("bad request", None, false)]
fn error_facts_behavior_tests_behavior(
    #[case] message: &str,
    #[case] status: Option<u16>,
    #[case] expected: bool,
) {
    assert_eq!(
        ExceptionCheckers::is_error_str_rate_limit(message, status),
        expected
    );
}

#[rstest]
#[case::known("this model's maximum context length is 10", true)]
#[case::validation("string_above_max_length", false)]
#[case::validation_with_context_phrase(
    "string_above_max_length this model's maximum context length is 10",
    false
)]
#[case::ordinary("bad request", false)]
fn error_facts_behavior_tests_behavior_context_window(
    #[case] message: &str,
    #[case] expected: bool,
) {
    assert_eq!(ExceptionCheckers::context_window(message), expected);
}

#[test]
fn error_facts_behavior_tests_behavior_proxy_reconstructs_empty_response_headers() {
    let facts = ProviderErrorFacts {
        status_code: Some(502),
        headers: Some(BTreeMap::from([(
            String::from("retry-after"),
            String::from("1"),
        )])),
        response_headers: Some(BTreeMap::new()),
        ..ProviderErrorFacts::empty()
    };
    let response = litellm_proxy_response(&facts, "litellm_proxy");
    assert_eq!(
        response.and_then(|value| value.headers.get("retry-after").cloned()),
        Some(String::from("1"))
    );
    let text = "a".repeat(16 * 1024 + 1);
    assert_eq!(bounded_text(&text).len(), 16 * 1024);
}
