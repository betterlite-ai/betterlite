#![cfg(test)]

use super::*;

#[rstest]
#[case::api("APIError")]
#[case::budget("BudgetExceededError")]
#[case::event("ErrorEventError")]
#[case::schema("JSONSchemaValidationError")]
#[case::openai("OpenAIError")]
#[case::router("RouterRateLimitError")]
#[case::vector_store("VectorStoreSearchError")]
fn error_facts_behavior_tests_behavior(#[case] name: &str) {
    let text = format!("litellm.{name}");
    assert!(matches!(
        extract_exception(None, &text, "model", "provider", None),
        Err(ParseError::Constructor { name: error_name }) if error_name == name
    ));
}
