use crate::bedrock_content::continuation::{
    assistant_continue_message, assistant_message_or_continue, insert_assistant_continue,
    user_message_block_or_continue,
};
use crate::bedrock_content::empty_blocks::skip_empty_text_blocks;
use crate::bedrock_content::error::ContentError;

use super::prelude::{json, object};

const DEFAULT_ASSISTANT: &str =
    r#"{"role":"assistant","content":[{"type":"text","text":"Please continue."}]}"#;
const USER_DEFAULT_CONTINUE: &str = r#"{"role":"user","content":"Please continue."}"#;

#[test]
fn bedrock_content_behavior_tests_behavior_insert_adds_nothing_without_modify_params() {
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    assert_eq!(insert_assistant_continue(&messages, None, false), messages);
}

#[test]
fn bedrock_content_behavior_tests_behavior_insert_adds_the_default_with_modify_params() {
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let inserted = insert_assistant_continue(&messages, None, true);
    assert_eq!(inserted.len(), 2);
    assert_eq!(
        inserted.get(1),
        Some(&json(
            r#"{"role":"assistant","content":[{"text":"Please continue."}]}"#
        ))
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_insert_turns_a_string_into_a_text_block() {
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let inserted = insert_assistant_continue(&messages, Some(&json(r#""go on""#)), false);
    assert_eq!(
        inserted.get(1),
        Some(&json(
            r#"{"role":"assistant","content":[{"text":"go on"}]}"#
        ))
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_insert_joins_a_dict_content_list() {
    let messages = Vec::new();
    let continue_message = json(
        r#"{"role":"assistant","content":[{"type":"text","text":"carry "},{"type":"text","text":"on"}]}"#,
    );
    let inserted = insert_assistant_continue(&messages, Some(&continue_message), false);
    assert_eq!(
        inserted.first(),
        Some(&json(
            r#"{"role":"assistant","content":[{"text":"carry on"}]}"#
        ))
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_insert_turns_a_contentless_dict_into_empty_text() {
    let inserted = insert_assistant_continue(&[], Some(&json(r#"{"role":"assistant"}"#)), false);
    assert_eq!(
        inserted.first(),
        Some(&json(r#"{"role":"assistant","content":[{"text":""}]}"#))
    );
}

#[rstest::rstest]
#[case::number("5")]
#[case::list("[]")]
fn bedrock_content_behavior_tests_behavior_insert_ignores_a_continue_of_another_type(
    #[case] continue_json: &str,
) {
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    assert_eq!(
        insert_assistant_continue(&messages, Some(&json(continue_json)), true),
        messages
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_insert_treats_a_null_continue_like_an_absent_one() {
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let inserted = insert_assistant_continue(&messages, Some(&json("null")), true);
    assert_eq!(
        inserted.get(1),
        Some(&json(
            r#"{"role":"assistant","content":[{"text":"Please continue."}]}"#
        ))
    );
}

#[rstest::rstest]
#[case::absent(None)]
#[case::null(Some("null"))]
#[case::empty_dict(Some("{}"))]
#[case::empty_string(Some(r#""""#))]
#[case::number(Some("5"))]
fn bedrock_content_behavior_tests_behavior_a_missing_or_empty_continue_resolves_to_default(
    #[case] continue_json: Option<&str>,
) {
    let continue_message = continue_json.map(json);
    assert_eq!(
        assistant_continue_message(continue_message.as_ref()),
        object(DEFAULT_ASSISTANT)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_string_continue_becomes_assistant_content() {
    assert_eq!(
        assistant_continue_message(Some(&json(r#""next""#))),
        object(r#"{"role":"assistant","content":"next"}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_dict_continue_is_copied_with_its_fields() {
    let continue_message = json(r#"{"role":"assistant","content":null,"name":"n"}"#);
    assert_eq!(
        assistant_continue_message(Some(&continue_message)),
        object(r#"{"role":"assistant","content":null,"name":"n"}"#)
    );
}

#[rstest::rstest]
#[case::blank(
    r#"{"role":"user","content":"   "}"#,
    r#"{"role":"user","content":"   "}"#
)]
#[case::empty_text_list(
    r#"{"role":"user","content":[{"type":"text","text":" "}]}"#,
    r#"{"role":"user","content":[]}"#
)]
#[case::absent_content(r#"{"role":"user"}"#, r#"{"role":"user"}"#)]
#[case::null_content(
    r#"{"role":"user","content":null}"#,
    r#"{"role":"user","content":null}"#
)]
fn bedrock_content_behavior_tests_behavior_user_continuation_without_modify_params_uses_skip(
    #[case] message_json: &str,
    #[case] expected_json: &str,
) {
    let message = json(message_json);
    assert_eq!(
        user_message_block_or_continue(&message, None, false).unwrap(),
        json(expected_json)
    );
}

#[rstest::rstest]
#[case::blank(r#"{"role":"user","content":"   "}"#)]
#[case::absent(r#"{"role":"user"}"#)]
#[case::null(r#"{"role":"user","content":null}"#)]
#[case::empty_list(r#"{"role":"user","content":[]}"#)]
#[case::python_separator(r#"{"role":"user","content":"\u001c"}"#)]
fn bedrock_content_behavior_tests_behavior_user_continuation_replaces_blank_content(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    let continued = user_message_block_or_continue(&message, None, true).unwrap();
    assert_eq!(continued, json(USER_DEFAULT_CONTINUE));
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_null_user_continue_is_absent() {
    let message = json(r#"{"role":"user","content":"   "}"#);
    let explicit_null = json("null");
    let continued = user_message_block_or_continue(&message, Some(&explicit_null), false).unwrap();
    assert_eq!(continued, message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_null_assistant_continue_is_absent() {
    let message = json(r#"{"role":"assistant","content":"   "}"#);
    let explicit_null = json("null");
    let continued = assistant_message_or_continue(&message, Some(&explicit_null), false).unwrap();
    assert_eq!(continued, message);
}

#[rstest::rstest]
#[case::null_other_field(r#"{"role":"assistant","content":"   ","tool_calls":null}"#)]
#[case::named_message(r#"{"role":"assistant","content":"   ","name":"bob"}"#)]
fn bedrock_content_behavior_tests_behavior_name_and_null_fields_are_not_content(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    assert_eq!(skip_empty_text_blocks(&message).unwrap(), message);
    let continued = assistant_message_or_continue(&message, None, true).unwrap();
    assert_eq!(continued, json(DEFAULT_ASSISTANT));
}

#[test]
fn bedrock_content_behavior_tests_behavior_user_continuation_copies_a_custom_message() {
    let message = json(r#"{"role":"user","content":"   "}"#);
    let continue_message = json(r#"{"role":"user","content":"carry on","name":"helper"}"#);
    assert_eq!(
        user_message_block_or_continue(&message, Some(&continue_message), false).unwrap(),
        json(r#"{"role":"user","content":"carry on","name":"helper"}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_user_continuation_keeps_a_plain_string() {
    let message = json(r#"{"role":"user","content":"hi"}"#);
    assert_eq!(
        user_message_block_or_continue(&message, None, true).unwrap(),
        message
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_user_continuation_replaces_the_first_empty_text() {
    let message = json(
        r#"{"role":"user","name":"bob","content":[{"type":"text","text":" "},{"type":"text","text":" "},{"type":"image_url","image_url":{"url":"x"}}]}"#,
    );
    assert_eq!(
        user_message_block_or_continue(&message, None, true).unwrap(),
        json(
            r#"{"role":"user","name":"bob","content":[{"type":"text","text":"Please continue."},{"type":"text","text":" "},{"type":"image_url","image_url":{"url":"x"}}]}"#
        )
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_user_continuation_keeps_a_list_without_empty_text() {
    let message = json(r#"{"role":"user","content":[{"type":"text","text":"hi"}]}"#);
    assert_eq!(
        user_message_block_or_continue(&message, None, true).unwrap(),
        message
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_python_separator_text_is_replaced() {
    let message = json(r#"{"role":"user","content":[{"type":"text","text":"\u001c"}]}"#);
    let continued = user_message_block_or_continue(&message, None, true).unwrap();
    assert_eq!(
        continued,
        json(r#"{"role":"user","content":[{"type":"text","text":"Please continue."}]}"#)
    );
}

#[rstest::rstest]
#[case::object(r#"{"role":"user","content":{"a":1}}"#)]
#[case::number(r#"{"role":"user","content":5}"#)]
fn bedrock_content_behavior_tests_behavior_user_continuation_rejects_unknown_content(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    let rejected = user_message_block_or_continue(&message, None, true);
    assert_eq!(rejected, Err(ContentError::UnsupportedContent));
}

#[test]
fn bedrock_content_behavior_tests_behavior_user_continuation_rejects_a_non_object_message() {
    assert_eq!(
        user_message_block_or_continue(&json("5"), None, false),
        Err(ContentError::MessageNotObject)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_user_continuation_rejects_a_string_continue() {
    let message = json(r#"{"role":"user","content":[]}"#);
    assert_eq!(
        user_message_block_or_continue(&message, Some(&json(r#""custom""#)), false),
        Err(ContentError::ContinueMessageNotObject)
    );
}

#[rstest::rstest]
#[case::missing_type(r#"{"role":"user","content":[{"text":"x"}]}"#, ContentError::PartWithoutType { index: 0 })]
#[case::missing_text(
    r#"{"role":"user","content":[{"type":"text"}]}"#,
    ContentError::TextPartWithoutText { index: 0 }
)]
#[case::null_text(
    r#"{"role":"user","content":[{"type":"text","text":null}]}"#,
    ContentError::TextPartNotText { index: 0 }
)]
fn bedrock_content_behavior_tests_behavior_user_continuation_part_failures(
    #[case] message_json: &str,
    #[case] expected: ContentError,
) {
    let message = json(message_json);
    assert_eq!(
        user_message_block_or_continue(&message, None, true),
        Err(expected)
    );
}

#[rstest::rstest]
#[case::absent(r#"{"role":"assistant"}"#)]
#[case::null(r#"{"role":"assistant","content":null}"#)]
fn bedrock_content_behavior_tests_behavior_assistant_continuation_keeps_missing_content(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    let continued = assistant_message_or_continue(&message, None, true).unwrap();
    assert_eq!(continued, message);
}

#[rstest::rstest]
#[case::blank_tool_calls(r#"{"role":"assistant","content":" ","tool_calls":[{"id":"call-1"}]}"#)]
#[case::empty_list(r#"{"role":"assistant","content":[]}"#)]
#[case::all_empty_list(r#"{"role":"assistant","content":[{"type":"text","text":" "}]}"#)]
fn bedrock_content_behavior_tests_behavior_assistant_without_modify_params_uses_skip(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    let continued = assistant_message_or_continue(&message, None, false).unwrap();
    assert_eq!(
        continued.as_object().unwrap().get("content"),
        Some(&json("null"))
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_assistant_blank_with_tool_calls_becomes_null() {
    let message = json(r#"{"role":"assistant","content":" ","tool_calls":[{"id":"call-1"}]}"#);
    let continue_message = json(r#""carry on""#);
    assert_eq!(
        assistant_message_or_continue(&message, Some(&continue_message), false).unwrap(),
        json(r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call-1"}]}"#)
    );
}

#[rstest::rstest]
#[case::default(None, DEFAULT_ASSISTANT)]
#[case::custom_string(Some(r#""carry on""#), r#"{"role":"assistant","content":"carry on"}"#)]
#[case::custom_dict(
    Some(r#"{"role":"assistant","content":"carry on","name":"n"}"#),
    r#"{"role":"assistant","content":"carry on","name":"n"}"#
)]
fn bedrock_content_behavior_tests_behavior_assistant_blank_string_gets_the_continue_message(
    #[case] continue_json: Option<&str>,
    #[case] expected_json: &str,
) {
    let message = json(r#"{"role":"assistant","content":" "}"#);
    let continue_message = continue_json.map(json);
    assert_eq!(
        assistant_message_or_continue(&message, continue_message.as_ref(), true).unwrap(),
        json(expected_json)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_assistant_empty_list_gets_the_continue_text() {
    let message = json(r#"{"role":"assistant","content":[]}"#);
    assert_eq!(
        assistant_message_or_continue(&message, None, true).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"Please continue."}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_a_python_separator_string_gets_continue() {
    let message = json(r#"{"role":"assistant","content":"\u001c"}"#);
    let continued = assistant_message_or_continue(&message, None, true).unwrap();
    assert_eq!(continued, json(DEFAULT_ASSISTANT));
}

#[test]
fn bedrock_content_behavior_tests_behavior_assistant_mixed_list_is_filtered() {
    let message = json(
        r#"{"role":"assistant","content":[{"type":"text","text":""},{"type":"text","text":"keep"}]}"#,
    );
    assert_eq!(
        assistant_message_or_continue(&message, None, true).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"keep"}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_assistant_continuation_keeps_a_plain_string() {
    let message = json(r#"{"role":"assistant","content":"hi"}"#);
    assert_eq!(
        assistant_message_or_continue(&message, None, true).unwrap(),
        message
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_assistant_continuation_rejects_unknown_content() {
    let message = json(r#"{"role":"assistant","content":{"a":1}}"#);
    assert_eq!(
        assistant_message_or_continue(&message, None, true),
        Err(ContentError::UnsupportedContent)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_assistant_continuation_rejects_a_non_object_message() {
    assert_eq!(
        assistant_message_or_continue(&json("5"), None, true),
        Err(ContentError::MessageNotObject)
    );
}
