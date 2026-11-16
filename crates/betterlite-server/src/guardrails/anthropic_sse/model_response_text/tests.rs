#![cfg(test)]

use super::model_response_text;
use crate::guardrails::anthropic_sse::tests::support::record;

#[test]
fn model_response_text_joins_the_text_of_every_choice() {
    let response = record(
        r#"{"id": "a", "choices": [{"message": {"content": "Hello, "}}, {"message": {"content": "world"}}]}"#,
    );
    assert_eq!(model_response_text(&response), "Hello, world");
}

#[test]
fn model_response_text_skips_a_choice_without_text() {
    let response = record(
        r#"{"id": "a", "choices": [{"message": {"content": null, "tool_calls": [{"id": "t"}]}}, {"message": {"content": "kept"}}]}"#,
    );
    assert_eq!(model_response_text(&response), "kept");
}

#[test]
fn model_response_text_is_empty_without_choices() {
    assert_eq!(model_response_text(&record(r#"{"id": "a"}"#)), "");
}
