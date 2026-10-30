#![cfg(test)]

use super::*;

#[rstest]
#[case::api_connection("APIConnectionError")]
#[case::api_response_validation("APIResponseValidationError")]
#[case::invalid_request("InvalidRequestError")]
fn error_facts_behavior_tests_behavior(#[case] name: &str) {
    let text = format!("litellm.{name}");
    let facts = extract_exception(None, &text, "model", "provider", None)
        .unwrap()
        .expect("plain exceptions should be constructible");
    assert_eq!(facts.name, name);
    assert!(facts.response.is_none());
    assert!(facts.body.is_none());
}
