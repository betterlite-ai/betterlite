#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::*;

#[rstest]
#[case::phrase("rate limit reached", None, true)]
#[case::number_with_429("429", Some(429), true)]
#[case::number_with_other_status("429", Some(400), false)]
#[case::number_inside_word("429abc", None, false)]
#[case::number_inside_long_number("1429", None, false)]
#[case::number_after_underscore("x_429", None, false)]
#[case::number_before_underscore("429_", None, false)]
#[case::service_tier("service tier capacity exceeded", Some(400), true)]
#[case::hyphenated("rate-limit reached", Some(400), true)]
#[case::ordinary("bad request", None, false)]
#[case::repeated_separators("rate--limit", None, true)]
#[case::no_separator("ratelimit", None, true)]
#[case::tab_separator("rate\tlimit", None, true)]
#[case::unicode_word_before("\u{e9}429", None, false)]
#[case::unicode_word_after("429\u{e9}", None, false)]
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
#[case::invalid_user_with_context_phrase(
    "invalid 'user' this model's maximum context length is 10",
    true
)]
#[case::ordinary("bad request", false)]
#[case::current_length_without_while_limit("current length is 500", false)]
#[case::while_limit_without_current_length("while limit is 400", false)]
#[case::current_length_and_while_limit("current length is 500 while limit is 400", true)]
#[case::maximum_input_length_without_tokens("maximum input length is 10", false)]
#[case::tokens_without_maximum_input_length("some tokens here", false)]
#[case::maximum_input_length_with_tokens("maximum input length is 10 tokens", true)]
fn error_facts_behavior_tests_behavior_context_window(
    #[case] message: &str,
    #[case] expected: bool,
) {
    assert_eq!(ExceptionCheckers::context_window(message), expected);
}

#[rstest]
#[case::content_policy("content_policy_violation", true)]
#[case::responsible_ai("ResponsibleAIPolicyViolation", true)]
#[case::ordinary("provider rejected the request", false)]
fn error_facts_behavior_tests_behavior_azure_policy(#[case] message: &str, #[case] expected: bool) {
    assert_eq!(ExceptionCheckers::azure_policy(message), expected);
}

#[test]
fn error_facts_behavior_tests_behavior_proxy_reconstructs_empty_response_headers() {
    let facts = ProviderErrorFacts {
        status_code: Some(502),
        headers: Some(BTreeMap::from([(
            String::from("retry-after"),
            String::from("1"),
        )])),
        response: Some(ResponseFacts {
            status_code: 502,
            request: None,
            headers: BTreeMap::new(),
        }),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        litellm_response_headers: None,
        request: None,
    };
    let response = litellm_proxy_response(&facts, "litellm_proxy");
    assert_eq!(
        response.and_then(|value| value.headers.get("retry-after").cloned()),
        Some(String::from("1"))
    );
    let text = "a".repeat(16 * 1024 + 1);
    assert_eq!(bounded_text(&text).len(), 16 * 1024);
}

#[test]
fn error_facts_behavior_tests_behavior_proxy_keeps_non_proxy_empty_headers() {
    let facts = ProviderErrorFacts {
        status_code: Some(502),
        response: Some(ResponseFacts {
            status_code: 502,
            headers: BTreeMap::new(),
            request: None,
        }),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        litellm_response_headers: None,
        request: None,
    };
    assert_eq!(
        litellm_proxy_response(&facts, "openai").map(|response| response.status_code),
        Some(502)
    );
}

#[test]
fn error_facts_behavior_tests_behavior_proxy_ignores_fallback_headers_for_non_proxy() {
    let facts = ProviderErrorFacts {
        status_code: Some(502),
        response: Some(ResponseFacts {
            status_code: 502,
            headers: BTreeMap::new(),
            request: None,
        }),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: Some(BTreeMap::from([(
            String::from("retry-after"),
            String::from("1"),
        )])),
        litellm_response_headers: None,
        request: None,
    };
    assert!(
        litellm_proxy_response(&facts, "openai")
            .is_some_and(|response| response.headers.is_empty())
    );
}

#[test]
fn error_facts_behavior_tests_behavior_body_boundaries_are_inclusive() {
    let empty = ProviderErrorBody {
        message: Field::Value(String::new()),
        error: Field::Missing,
        code: Field::Missing,
    };
    let overhead = serde_json::to_vec(&empty).unwrap().len();
    let body = ProviderErrorBody {
        message: Field::Value("a".repeat(MAX_BODY_BYTES - overhead)),
        error: Field::Missing,
        code: Field::Missing,
    };
    assert_eq!(serde_json::to_vec(&body).unwrap().len(), MAX_BODY_BYTES);
    assert!(bounded_body(&body).is_some());
    let oversized = ProviderErrorBody {
        message: Field::Value(format!(
            "{}a",
            body.message.as_ref().map_or("", String::as_str)
        )),
        error: Field::Missing,
        code: Field::Missing,
    };
    assert!(bounded_body(&oversized).is_none());
}

#[test]
fn proxy_without_response_is_absent() {
    let error = ProviderErrorFacts {
        status_code: Some(502),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    assert_eq!(litellm_proxy_response(&error, "openai"), None);
}

mod azure_policy_gate_cases;

mod proxy_header_gate_cases;
