#![cfg(test)]
#![cfg(test)]

use std::collections::BTreeMap;
use std::iter::repeat_n;

use crate::facts::ProviderErrorBody;
use rstest::rstest;

use super::*;

#[rstest]
#[case::body_message(Some(ProviderErrorBody { message: Some("direct".to_owned()), error: None, code: None }), Some("direct"))]
#[case::nested_message(Some(ProviderErrorBody { message: None, error: Some(Box::new(ProviderErrorBody { message: Some("nested".to_owned()), error: None, code: None })), code: None }), Some("nested"))]
#[case::missing_message(Some(ProviderErrorBody { message: None, error: None, code: None }), None)]
#[case::empty_message_uses_nested(Some(ProviderErrorBody { message: Some(String::new()), error: Some(Box::new(ProviderErrorBody { message: Some("nested".to_owned()), error: None, code: None })), code: None }), Some("nested"))]
#[case::null_body(None, None)]
fn error_facts_behavior_tests_behavior(
    #[case] body: Option<ProviderErrorBody>,
    #[case] expected: Option<&str>,
) {
    let facts = ProviderErrorFacts {
        body,
        ..ProviderErrorFacts::empty()
    };
    assert_eq!(get_error_message(&facts).as_deref(), expected);
}

#[test]
fn error_facts_behavior_tests_behavior_preserves_header_precedence() {
    let facts = ProviderErrorFacts {
        headers: Some(BTreeMap::from([(String::from("a"), String::from("one"))])),
        response_headers: Some(BTreeMap::from([(String::from("a"), String::from("two"))])),
        litellm_response_headers: Some(BTreeMap::from([(
            String::from("a"),
            String::from("three"),
        )])),
        ..ProviderErrorFacts::empty()
    };
    assert_eq!(
        get_response_headers(&facts).and_then(|headers| headers.get("a").cloned()),
        Some(String::from("one"))
    );
}

#[test]
fn error_facts_behavior_tests_behavior_empty_headers_fall_through() {
    let facts = ProviderErrorFacts {
        headers: Some(BTreeMap::new()),
        response_headers: Some(BTreeMap::from([(String::from("a"), String::from("two"))])),
        litellm_response_headers: Some(BTreeMap::from([(
            String::from("a"),
            String::from("three"),
        )])),
        ..ProviderErrorFacts::empty()
    };
    assert_eq!(
        get_response_headers(&facts).and_then(|headers| headers.get("a").cloned()),
        Some(String::from("two"))
    );
    let only_empty = ProviderErrorFacts {
        headers: Some(BTreeMap::new()),
        response_headers: Some(BTreeMap::new()),
        litellm_response_headers: Some(BTreeMap::new()),
        ..ProviderErrorFacts::empty()
    };
    assert_eq!(get_response_headers(&only_empty), None);
}

#[test]
fn error_facts_behavior_tests_behavior_accepts_only_exception_body_kwargs() {
    let candidates = BTreeMap::from([
        (
            String::from("response"),
            ProviderErrorBody {
                message: None,
                error: None,
                code: None,
            },
        ),
        (
            String::from("body"),
            ProviderErrorBody {
                message: None,
                error: None,
                code: None,
            },
        ),
        (
            String::from("message"),
            ProviderErrorBody {
                message: None,
                error: None,
                code: None,
            },
        ),
    ]);
    let accepted = accepted_init_kwargs(&candidates);
    assert_eq!(accepted.len(), 2);
    assert!(accepted.contains_key("response"));
    assert!(accepted.contains_key("body"));
}

#[rstest]
#[case::numeric("{\"error\":{\"code\":429}}", Some(429))]
#[case::string("{\"error\":{\"code\":\"429\"}}", Some(429))]
#[case::malformed("not-json", None)]
#[case::wrong_shape("{\"code\":429}", None)]
fn error_facts_behavior_tests_behavior_body_code(
    #[case] body: &str,
    #[case] expected: Option<i64>,
) {
    assert_eq!(get_body_error_code(body), expected);
}

#[rstest]
#[case::valid(
    "prefix litellm.ContextWindowExceededError suffix",
    Some("ContextWindowExceededError")
)]
#[case::missing_name("litellm.Error", None)]
#[case::missing_error_suffix("litellm.ContextWindowExceeded", None)]
#[case::not_litellm("provider error", None)]
fn error_facts_behavior_tests_behavior_exception_name(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        extract_exception(None, text, "model", "provider", None).map(|facts| facts.name),
        expected.map(str::to_owned)
    );
}

#[test]
fn error_facts_behavior_tests_behavior_parser_bounds_input() {
    let oversized = vec![b' '; 64 * 1024 + 1];
    assert_eq!(
        parse_provider_error(&oversized)
            .err()
            .map(|error| error.to_string()),
        Some(String::from("provider error body exceeds the bounded size"))
    );
    let prefix = br#"{"message":"a"}"#;
    let padding = 64 * 1024 - prefix.len();
    let mut body = Vec::with_capacity(64 * 1024);
    body.extend_from_slice(prefix);
    body.extend(repeat_n(b' ', padding));
    assert_eq!(body.len(), 64 * 1024);
    drop(parse_provider_error(&body).unwrap());
}

#[test]
fn error_facts_behavior_tests_behavior_extracts_all_facts() {
    let facts = extract_facts(br#"{"message":"provider failed"}"#, Some(502));
    assert_eq!(facts.status_code, Some(502));
    assert!(!facts.status_code_is_synthesized);
    assert_eq!(facts.message.as_deref(), Some("provider failed"));
    assert_eq!(
        facts.body.and_then(|body| body.message),
        Some(String::from("provider failed"))
    );
    let synthesized = extract_facts(br#"{"message":"provider failed"}"#, None);
    assert_eq!(synthesized.status_code, None);
    assert!(synthesized.status_code_is_synthesized);
}

#[test]
fn replays_the_provider_error_corpus() {
    let malformed = include_bytes!("../../fuzz/corpus/provider_error/malformed");
    let nested = include_bytes!("../../fuzz/corpus/provider_error/nested");
    drop(parse_provider_error(malformed).unwrap_err());
    drop(parse_provider_error(nested).unwrap());
}
