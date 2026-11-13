#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::extract_output;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::ExtractedContent;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{
    guardrail, json, object,
};

fn extract(choice: JsonValue) -> ExtractedContent {
    let mut content = ExtractedContent::default();
    extract_output(&guardrail(choice), 2, &mut content);
    content
}

#[rstest]
#[case::completed_string(json(r#"{"message": {"content": "hi"}}"#), &["hi"], &[(2, None)])]
#[case::streamed_string(json(r#"{"delta": {"content": "hi"}}"#), &["hi"], &[(2, None)])]
#[case::empty_string_is_skipped(json(r#"{"message": {"content": ""}}"#), &[], &[])]
#[case::null_content(json(r#"{"message": {"content": null}}"#), &[], &[])]
#[case::absent_content(json(r#"{"delta": {"role": "assistant"}}"#), &[], &[])]
#[case::empty_list(json(r#"{"message": {"content": []}}"#), &[], &[])]
#[case::list_text_keeps_its_item_index(
    json(r#"{"message": {"content": [{"type": "image_url", "image_url": {"url": "u"}}, {"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}}"#),
    &["a", "b"],
    &[(2, Some(1)), (2, Some(2))],
)]
#[case::empty_and_null_item_text_is_skipped(json(r#"{"delta": {"content": [{"text": ""}, {"text": null}, {}]}}"#), &[], &[])]
#[case::choice_of_an_unknown_type(json(r#"{"index": 0, "text": "legacy"}"#), &[], &[])]
fn text_is_extracted_with_the_choice_index(
    #[case] choice: JsonValue,
    #[case] expected: &[&str],
    #[case] mappings: &[(usize, Option<usize>)],
) {
    let content = extract(choice);
    assert_eq!(content.texts, expected);
    assert_eq!(content.text_mappings, mappings);
}

#[rstest]
#[case::url_object(json(r#"{"message": {"content": [{"type": "image_url", "image_url": {"url": "u"}}]}}"#), &["u"])]
#[case::bare_string_is_not_read(json(r#"{"message": {"content": [{"type": "image_url", "image_url": "u"}]}}"#), &[])]
#[case::empty_url(json(r#"{"message": {"content": [{"type": "image_url", "image_url": {"url": ""}}]}}"#), &[])]
#[case::other_item_type(json(r#"{"message": {"content": [{"type": "input_image", "image_url": {"url": "u"}}]}}"#), &[])]
#[case::image_in_a_string_choice_is_not_possible(json(r#"{"delta": {"content": "u"}}"#), &[])]
fn images_come_from_url_objects_in_list_content(
    #[case] choice: JsonValue,
    #[case] expected: &[&str],
) {
    assert_eq!(extract(choice).images, expected);
}

#[test]
fn tool_calls_keep_their_position_and_drop_invalid_ones() {
    let content = extract(json(
        r#"{
        "message": {"content": "x", "tool_calls": [{"id": "1"}, "bad", null, {"id": "4"}]}
    }"#,
    ));
    assert_eq!(
        content.tool_calls,
        [
            object(json(r#"{"id": "1"}"#)),
            object(json(r#"{"id": "4"}"#))
        ]
    );
    assert_eq!(content.tool_call_mappings, [(2, 0), (2, 3)]);
}

#[test]
fn streamed_tool_calls_are_read_from_the_delta() {
    let content = extract(json(
        r#"{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{"}}]}}"#,
    ));
    assert_eq!(content.tool_call_mappings, [(2, 0)]);
}

#[rstest]
#[case::null(json(r#"{"message": {"tool_calls": null}}"#))]
#[case::not_a_list(json(r#"{"message": {"tool_calls": {"id": "1"}}}"#))]
#[case::legacy_function_call_is_not_a_tool_call(json(
    r#"{"delta": {"function_call": {"name": "f"}}}"#
))]
fn tool_calls_that_are_not_a_list_give_nothing(#[case] choice: JsonValue) {
    assert!(extract(choice).tool_calls.is_empty());
}
