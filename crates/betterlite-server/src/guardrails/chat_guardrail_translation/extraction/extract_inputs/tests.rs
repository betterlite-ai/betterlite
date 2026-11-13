#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{ScanScope, ScopeFlags};
use rstest::rstest;

use super::extract_inputs;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::ExtractedContent;
use crate::guardrails::chat_guardrail_translation::extraction::fixtures::{
    guardrail, json, object,
};

fn extract(message: JsonValue, message_index: usize, flags: ScopeFlags) -> ExtractedContent {
    let mut content = ExtractedContent::default();
    extract_inputs(&guardrail(message), message_index, &flags, &mut content);
    content
}

fn texts(content: &ExtractedContent) -> Vec<&str> {
    content.texts.iter().map(String::as_str).collect()
}

#[rstest]
#[case::string_content(json(r#"{"role": "user", "content": "hi"}"#), &["hi"], &[(3, None)])]
#[case::empty_string_is_extracted(json(r#"{"role": "user", "content": ""}"#), &[""], &[(3, None)])]
#[case::list_text_items_keep_their_position(
    json(r#"{"content": [{"type": "image_url", "image_url": {"url": "u"}}, {"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}"#),
    &["a", "b"],
    &[(3, Some(1)), (3, Some(2))],
)]
#[case::empty_item_text_is_extracted(json(r#"{"content": [{"type": "text", "text": ""}]}"#), &[""], &[(3, Some(0))])]
#[case::null_item_text_is_skipped(json(r#"{"content": [{"type": "text", "text": null}, {"type": "text"}]}"#), &[], &[])]
#[case::null_content(json(r#"{"content": null}"#), &[], &[])]
#[case::absent_content(json(r#"{"role": "user"}"#), &[], &[])]
#[case::content_of_another_type(json(r#"{"content": 5}"#), &[], &[])]
#[case::list_item_that_is_not_an_object(json(r#"{"content": ["a", 1]}"#), &[], &[])]
fn text_is_extracted_with_its_position(
    #[case] message: JsonValue,
    #[case] expected: &[&str],
    #[case] mappings: &[(usize, Option<usize>)],
) {
    let content = extract(message, 3, ScopeFlags::default());
    assert_eq!(texts(&content), expected);
    assert_eq!(content.text_mappings, mappings);
}

#[rstest]
#[case::url_object(json(r#"{"content": [{"type": "image_url", "image_url": {"url": "u"}}]}"#), &["u"])]
#[case::url_string(json(r#"{"content": [{"type": "image_url", "image_url": "u"}]}"#), &["u"])]
#[case::empty_url_string_is_kept(json(r#"{"content": [{"type": "image_url", "image_url": ""}]}"#), &[""])]
#[case::empty_url_in_an_object_is_dropped(json(r#"{"content": [{"type": "image_url", "image_url": {"url": ""}}]}"#), &[])]
#[case::object_without_a_url(json(r#"{"content": [{"type": "image_url", "image_url": {}}]}"#), &[])]
#[case::missing_image_url(json(r#"{"content": [{"type": "image_url"}]}"#), &[])]
#[case::other_item_types_are_not_images(json(r#"{"content": [{"type": "input_image", "image_url": "u"}]}"#), &[])]
#[case::two_images(json(r#"{"content": [{"type": "image_url", "image_url": "a"}, {"type": "image_url", "image_url": {"url": "b"}}]}"#), &["a", "b"])]
fn images_are_extracted_from_list_content(#[case] message: JsonValue, #[case] expected: &[&str]) {
    let content = extract(message, 0, ScopeFlags::default());
    assert_eq!(content.images, expected);
    assert!(content.texts.is_empty());
}

#[test]
fn a_text_item_that_is_also_an_image_gives_both() {
    let content = extract(
        json(r#"{"content": [{"type": "image_url", "text": "t", "image_url": "u"}]}"#),
        0,
        ScopeFlags::default(),
    );
    assert_eq!(texts(&content), ["t"]);
    assert_eq!(content.images, ["u"]);
}

#[test]
fn tool_calls_keep_the_whole_object_and_their_position() {
    let content = extract(
        json(
            r#"{"role": "assistant", "content": "x", "tool_calls": [{"id": "1"}, "bad", null, {"id": "4"}]}"#,
        ),
        2,
        ScopeFlags::default(),
    );
    assert_eq!(
        content.tool_calls,
        [
            object(json(r#"{"id": "1"}"#)),
            object(json(r#"{"id": "4"}"#))
        ]
    );
    assert_eq!(content.tool_call_mappings, [(2, 0), (2, 3)]);
    assert_eq!(content.text_mappings, [(2, None)]);
}

#[rstest]
#[case::null(json(r#"{"tool_calls": null}"#))]
#[case::not_a_list(json(r#"{"tool_calls": {"id": "1"}}"#))]
#[case::absent(json(r#"{"role": "assistant"}"#))]
fn tool_calls_that_are_not_a_list_give_nothing(#[case] message: JsonValue) {
    assert!(
        extract(message, 0, ScopeFlags::default())
            .tool_calls
            .is_empty()
    );
}

#[rstest]
#[case::system_skipped(
    json(r#"{"role": "system", "content": "s"}"#),
    ScopeFlags { skip_system_message: true, ..ScopeFlags::default() },
    0,
)]
#[case::system_kept_by_default(
    json(r#"{"role": "system", "content": "s"}"#),
    ScopeFlags::default(),
    1
)]
#[case::role_is_compared_lowercase(
    json(r#"{"role": "SYSTEM", "content": "s"}"#),
    ScopeFlags { skip_system_message: true, ..ScopeFlags::default() },
    0,
)]
#[case::tool_skipped(
    json(r#"{"role": "tool", "content": "t", "tool_calls": [{"id": "1"}]}"#),
    ScopeFlags { skip_tool_message: true, ..ScopeFlags::default() },
    0,
)]
#[case::user_is_never_skipped_for_tools(
    json(r#"{"role": "user", "content": "u"}"#),
    ScopeFlags { skip_tool_message: true, skip_system_message: true, scan: ScanScope::AllMessages },
    1,
)]
#[case::only_tool_results_drops_users(
    json(r#"{"role": "user", "content": "u"}"#),
    ScopeFlags { scan: ScanScope::ToolResultsOnly, ..ScopeFlags::default() },
    0,
)]
#[case::only_tool_results_keeps_tools(
    json(r#"{"role": "tool", "content": "t"}"#),
    ScopeFlags { scan: ScanScope::ToolResultsOnly, ..ScopeFlags::default() },
    1,
)]
fn messages_outside_the_scope_are_not_extracted(
    #[case] message: JsonValue,
    #[case] flags: ScopeFlags,
    #[case] expected_texts: usize,
) {
    let content = extract(message, 0, flags);
    assert_eq!(content.texts.len(), expected_texts);
    assert_eq!(content.tool_calls.len(), 0);
}
