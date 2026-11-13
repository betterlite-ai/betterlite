#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::has_text_content;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

#[rstest]
#[case::message_text(json(r#"{"choices": [{"message": {"content": "hi"}}]}"#), true)]
#[case::delta_text(json(r#"{"choices": [{"delta": {"content": "hi"}}]}"#), true)]
#[case::message_tool_calls(
    json(r#"{"choices": [{"message": {"content": null, "tool_calls": [{"id": "1"}]}}]}"#),
    true
)]
#[case::delta_tool_calls(
    json(r#"{"choices": [{"delta": {"tool_calls": [{"id": "1"}]}}]}"#),
    true
)]
#[case::later_choice_has_text(
    json(r#"{"choices": [{"message": {"content": ""}}, {"message": {"content": "hi"}}]}"#),
    true
)]
#[case::empty_string(json(r#"{"choices": [{"message": {"content": ""}}]}"#), false)]
#[case::null_content(json(r#"{"choices": [{"message": {"content": null}}]}"#), false)]
#[case::empty_tool_calls(
    json(r#"{"choices": [{"message": {"content": null, "tool_calls": []}}]}"#),
    false
)]
#[case::null_tool_calls(json(r#"{"choices": [{"delta": {"tool_calls": null}}]}"#), false)]
#[case::tool_calls_that_are_not_a_list(
    json(r#"{"choices": [{"delta": {"tool_calls": {"id": "1"}}}]}"#),
    false
)]
#[case::list_content_is_not_text(
    json(r#"{"choices": [{"message": {"content": [{"text": "hi"}]}}]}"#),
    false
)]
#[case::choice_without_message_or_delta(json(r#"{"choices": [{"index": 0}]}"#), false)]
#[case::no_choices(json(r#"{"choices": []}"#), false)]
#[case::missing_choices(json(r#"{"id": "x"}"#), false)]
fn text_or_tool_calls_make_a_response_worth_scanning(
    #[case] response: JsonValue,
    #[case] expected: bool,
) {
    assert_eq!(has_text_content(&object(response)), expected);
}
