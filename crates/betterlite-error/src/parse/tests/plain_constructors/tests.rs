#![cfg(test)]

use super::*;
use crate::parse::constructors::ExceptionName;

#[rstest]
#[case::api_connection("APIConnectionError", ExceptionName::ApiConnection)]
#[case::api_response_validation("APIResponseValidationError", ExceptionName::ApiResponseValidation)]
#[case::invalid_request("InvalidRequestError", ExceptionName::InvalidRequest)]
fn error_facts_behavior_tests_behavior(#[case] name: &str, #[case] expected: ExceptionName) {
    let text = format!("litellm.{name}");
    let facts = extract_exception(None, &text, "model", "provider", None)
        .unwrap()
        .expect("plain exceptions should be constructible");
    assert_eq!(facts.name, expected);
    assert!(facts.response.is_none());
    assert!(facts.body.is_none());
}
