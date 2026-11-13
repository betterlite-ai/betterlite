#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::apply_output_texts;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::TextMapping;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{json, object};

fn rewrite(
    source: JsonValue,
    replies: &[&str],
    mappings: &[TextMapping],
) -> (Result<(), ApplyError>, JsonValue) {
    let mut response = object(source);
    let responses: Vec<String> = replies.iter().map(|text| (*text).to_owned()).collect();
    let result = apply_output_texts(&mut response, &responses, mappings);
    (result, serde_json::to_value(&response).unwrap())
}

#[rstest]
#[case::string_content(
    json(r#"{"id": "c1", "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "secret"}}]}"#),
    &["clean"],
    &[(0, None)],
    json(r#"{"id": "c1", "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "clean"}}]}"#),
)]
#[case::list_item_text(
    json(r#"{"choices": [{"message": {"content": [{"type": "text", "text": "secret"}, {"type": "text", "text": "kept"}]}}]}"#),
    &["clean"],
    &[(0, Some(0))],
    json(r#"{"choices": [{"message": {"content": [{"type": "text", "text": "clean"}, {"type": "text", "text": "kept"}]}}]}"#),
)]
#[case::each_mapping_picks_its_choice(
    json(r#"{"choices": [{"message": {"content": "a"}}, {"message": {"content": "b"}}, {"message": {"content": "c"}}]}"#),
    &["x", "z"],
    &[(2, None), (0, None)],
    json(r#"{"choices": [{"message": {"content": "z"}}, {"message": {"content": "b"}}, {"message": {"content": "x"}}]}"#),
)]
#[case::string_content_with_a_list_slot_is_untouched(
    json(r#"{"choices": [{"message": {"content": "keep"}}]}"#),
    &["clean"],
    &[(0, Some(0))],
    json(r#"{"choices": [{"message": {"content": "keep"}}]}"#),
)]
#[case::list_content_with_a_string_slot_is_untouched(
    json(r#"{"choices": [{"message": {"content": [{"text": "keep"}]}}]}"#),
    &["clean"],
    &[(0, None)],
    json(r#"{"choices": [{"message": {"content": [{"text": "keep"}]}}]}"#),
)]
#[case::null_content_is_skipped(
    json(r#"{"choices": [{"message": {"content": null, "tool_calls": []}}]}"#),
    &["clean"],
    &[(0, None)],
    json(r#"{"choices": [{"message": {"content": null, "tool_calls": []}}]}"#),
)]
#[case::choice_without_a_message_is_skipped(
    json(r#"{"choices": [{"delta": {"content": "keep"}}]}"#),
    &["clean"],
    &[(0, None)],
    json(r#"{"choices": [{"delta": {"content": "keep"}}]}"#),
)]
fn a_response_replaces_the_text_of_its_choice(
    #[case] response: JsonValue,
    #[case] responses: &[&str],
    #[case] mappings: &[TextMapping],
    #[case] expected: JsonValue,
) {
    let (result, rewritten) = rewrite(response, responses, mappings);
    assert_eq!(result, Ok(()));
    assert_eq!(rewritten, expected);
}

#[rstest]
#[case::response_without_mapping(
    json(r#"{"choices": [{"message": {"content": "a"}}]}"#),
    &["x", "y"],
    &[(0, None)],
    ApplyError::UnmappedResponse { response_index: 1 },
)]
#[case::choice_past_the_last(
    json(r#"{"choices": [{"message": {"content": "a"}}]}"#),
    &["x"],
    &[(1, None)],
    ApplyError::MissingEntry { index: 1 },
)]
#[case::response_without_choices(json("{}"), &["x"], &[(0, None)], ApplyError::MissingEntry { index: 0 })]
#[case::list_slot_past_the_last_item(
    json(r#"{"choices": [{"message": {"content": [{"text": "a"}]}}]}"#),
    &["x"],
    &[(0, Some(3))],
    ApplyError::MissingContentItem { index: 0, content_index: 3 },
)]
fn an_unplaceable_response_is_an_error(
    #[case] response: JsonValue,
    #[case] responses: &[&str],
    #[case] mappings: &[TextMapping],
    #[case] expected: ApplyError,
) {
    assert_eq!(rewrite(response, responses, mappings).0, Err(expected));
}
