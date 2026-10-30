#![cfg(test)]

use std::collections::BTreeMap;
use std::iter::repeat_n;
use std::{fs, path::PathBuf};

use crate::facts::presence::Field;
use crate::facts::{ProviderErrorBody, ResponseFacts};
use rstest::rstest;

use super::*;

#[rstest]
#[case::body_message(Some(ProviderErrorBody { message: Field::Value("direct".to_owned()), error: Field::Missing, code: Field::Missing }), Some("direct"))]
#[case::nested_message(Some(ProviderErrorBody { message: Field::Missing, error: Field::Value(Box::new(ProviderErrorBody { message: Field::Value("nested".to_owned()), error: Field::Missing, code: Field::Missing })), code: Field::Missing }), Some("nested"))]
#[case::missing_message(Some(ProviderErrorBody { message: Field::Missing, error: Field::Missing, code: Field::Missing }), None)]
#[case::empty_message_uses_nested(Some(ProviderErrorBody { message: Field::Value(String::new()), error: Field::Value(Box::new(ProviderErrorBody { message: Field::Value("nested".to_owned()), error: Field::Missing, code: Field::Missing })), code: Field::Missing }), Some("nested"))]
#[case::null_body(None, None)]
fn error_facts_behavior_tests_behavior(
    #[case] body: Option<ProviderErrorBody>,
    #[case] expected: Option<&str>,
) {
    let facts = ProviderErrorFacts {
        body,
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    let message = get_error_message(&facts);
    assert_eq!(message.as_deref(), expected);
}

#[test]
fn error_facts_behavior_tests_behavior_preserves_header_precedence() {
    let facts = ProviderErrorFacts {
        headers: Some(BTreeMap::from([(String::from("a"), String::from("one"))])),
        response: Some(ResponseFacts {
            status_code: 502,
            request: None,
            headers: BTreeMap::from([(String::from("a"), String::from("two"))]),
        }),
        litellm_response_headers: Some(BTreeMap::from([(
            String::from("a"),
            String::from("three"),
        )])),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        request: None,
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
        response: Some(ResponseFacts {
            status_code: 502,
            request: None,
            headers: BTreeMap::from([(String::from("a"), String::from("two"))]),
        }),
        litellm_response_headers: Some(BTreeMap::from([(
            String::from("a"),
            String::from("three"),
        )])),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        request: None,
    };
    assert_eq!(
        get_response_headers(&facts).and_then(|headers| headers.get("a").cloned()),
        Some(String::from("two"))
    );
    let only_empty = ProviderErrorFacts {
        headers: Some(BTreeMap::new()),
        response: Some(ResponseFacts {
            status_code: 502,
            request: None,
            headers: BTreeMap::new(),
        }),
        litellm_response_headers: Some(BTreeMap::new()),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        request: None,
    };
    assert_eq!(get_response_headers(&only_empty), Some(BTreeMap::new()));
}

#[test]
fn error_facts_behavior_tests_behavior_accepts_only_exception_body_kwargs() {
    let candidates = BTreeMap::from([
        (
            String::from("response"),
            ProviderErrorBody {
                message: Field::Missing,
                error: Field::Missing,
                code: Field::Missing,
            },
        ),
        (
            String::from("body"),
            ProviderErrorBody {
                message: Field::Missing,
                error: Field::Missing,
                code: Field::Missing,
            },
        ),
        (
            String::from("message"),
            ProviderErrorBody {
                message: Field::Missing,
                error: Field::Missing,
                code: Field::Missing,
            },
        ),
    ]);
    let accepted = accepted_init_kwargs(&["response", "body"], &candidates);
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
#[case::unknown("litellm.UnknownProviderFailureError", None)]
#[case::later_match("litellm.invalid litellm.BadRequestError", Some("BadRequestError"))]
#[case::suffix("litellm.BadRequestErrorSuffix", Some("BadRequestError"))]
fn error_facts_behavior_tests_behavior_exception_name(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        extract_exception(None, text, "model", "provider", None)
            .unwrap()
            .map(|facts| facts.name),
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
        facts.body.map(|body| body.message),
        Some(Field::Value(String::from("provider failed")))
    );
    let synthesized = extract_facts(br#"{"message":"provider failed"}"#, None);
    assert_eq!(synthesized.status_code, None);
    assert!(synthesized.status_code_is_synthesized);
}

#[rstest]
fn replays_the_provider_error_corpus(#[files("fuzz/corpus/provider_error/*")] path: PathBuf) {
    let malformed = include_bytes!("../../fuzz/corpus/provider_error/malformed");
    let nested = include_bytes!("../../fuzz/corpus/provider_error/nested");
    drop(parse_provider_error(malformed).unwrap_err());
    drop(parse_provider_error(nested).unwrap());
    let bytes = fs::read(path).unwrap();
    let _parsed = parse_provider_error(&bytes);
}

#[rstest]
#[case::missing("{}")]
#[case::null("{\"message\":null,\"error\":null,\"code\":null}")]
#[case::text("{\"message\":\"failed\",\"code\":\"code\"}")]
fn error_facts_behavior_tests_behavior_presence(#[case] source: &str) {
    let parsed = parse_provider_error(source.as_bytes()).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), source);
}

#[rstest]
#[case::unrelated_message("{\"message\":42,\"error\":{\"code\":429}}", Some(429))]
#[case::spaced_code("{\"error\":{\"code\":\" +429 \"}}", Some(429))]
fn error_facts_behavior_tests_behavior_code_fields(
    #[case] body: &str,
    #[case] expected: Option<i64>,
) {
    assert_eq!(get_body_error_code(body), expected);
}
