#![cfg(test)]
#![cfg(test)]

use std::collections::BTreeMap;

use crate::facts::{MAX_BODY_BYTES, ProviderErrorBody};
use rstest::rstest;

use super::*;

#[rstest]
#[case::body_message(Some(ProviderErrorBody { message: Some("direct".to_owned()), error: None, code: None }), Some("direct"))]
#[case::nested_message(Some(ProviderErrorBody { message: None, error: Some(Box::new(ProviderErrorBody { message: Some("nested".to_owned()), error: None, code: None })), code: None }), Some("nested"))]
#[case::missing_message(Some(ProviderErrorBody { message: None, error: None, code: None }), None)]
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

#[test]
fn error_facts_behavior_tests_behavior_parser_bounds_input() {
    let oversized = vec![b' '; MAX_BODY_BYTES + 1];
    assert_eq!(
        parse_provider_error(&oversized)
            .err()
            .map(|error| error.to_string()),
        Some(String::from("provider error body exceeds the bounded size"))
    );
}

#[test]
fn replays_the_provider_error_corpus() {
    let malformed = include_bytes!("../../fuzz/corpus/provider_error/malformed");
    let nested = include_bytes!("../../fuzz/corpus/provider_error/nested");
    drop(parse_provider_error(malformed).unwrap_err());
    drop(parse_provider_error(nested).unwrap());
}
