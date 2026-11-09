use crate::bedrock_content::empty_blocks::{process_empty_text_blocks, skip_empty_text_blocks};
use crate::bedrock_content::error::ContentError;

use super::prelude::{field, json};

#[rstest::rstest]
#[case::absent(r#"{"role":"assistant"}"#)]
#[case::null(r#"{"role":"assistant","content":null}"#)]
fn bedrock_content_behavior_tests_behavior_skip_keeps_missing_content(#[case] message_json: &str) {
    let message = json(message_json);
    assert_eq!(skip_empty_text_blocks(&message).unwrap(), message);
}

#[rstest::rstest]
#[case::plain(r#"{"role":"assistant","content":"hi"}"#)]
#[case::whitespace(r#"{"role":"user","content":"   "}"#)]
#[case::object(r#"{"role":"assistant","content":{"a":1}}"#)]
fn bedrock_content_behavior_tests_behavior_skip_keeps_non_empty_or_unknown_string_content(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    assert_eq!(skip_empty_text_blocks(&message).unwrap(), message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_keeps_a_blank_string_without_other_fields() {
    let message = json(r#"{"role":"assistant","content":"   "}"#);
    assert_eq!(skip_empty_text_blocks(&message).unwrap(), message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_nulls_a_blank_assistant_string_with_tool_calls() {
    let message = json(r#"{"role":"assistant","content":"   ","tool_calls":[{"id":"call-1"}]}"#);
    assert_eq!(
        skip_empty_text_blocks(&message).unwrap(),
        json(r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call-1"}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_keeps_a_blank_user_string() {
    let message = json(r#"{"role":"user","content":"   ","name":"bob"}"#);
    assert_eq!(skip_empty_text_blocks(&message).unwrap(), message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_drops_empty_text_parts() {
    let message = json(
        r#"{"role":"assistant","content":[{"type":"text","text":" "},{"type":"text","text":"\u001c"},{"type":"text","text":"keep"}]}"#,
    );
    assert_eq!(
        skip_empty_text_blocks(&message).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"keep"}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_treats_a_python_separator_as_blank() {
    let message = json(r#"{"role":"assistant","content":"\u001c","tool_calls":[{"id":"call-1"}]}"#);
    let skipped = skip_empty_text_blocks(&message).unwrap();
    assert_eq!(
        skipped,
        json(r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call-1"}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_drops_a_text_part_without_text() {
    let message = json(
        r#"{"role":"assistant","content":[{"type":"text"},{"type":"image_url","image_url":{"url":"x"}}]}"#,
    );
    assert_eq!(
        skip_empty_text_blocks(&message).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"image_url","image_url":{"url":"x"}}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_nulls_an_all_empty_assistant_list() {
    let message = json(
        r#"{"role":"assistant","content":[{"type":"text","text":" "},{"type":"text","text":""}]}"#,
    );
    assert_eq!(
        skip_empty_text_blocks(&message).unwrap(),
        json(r#"{"role":"assistant","content":null}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_keeps_an_empty_user_list() {
    let message = json(r#"{"role":"user","content":[]}"#);
    assert_eq!(
        skip_empty_text_blocks(&message).unwrap(),
        json(r#"{"role":"user","content":[]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_leaves_a_non_text_role_list_unchanged() {
    let message = json(r#"{"role":"system","content":[{"type":"text","text":""}]}"#);
    assert_eq!(skip_empty_text_blocks(&message).unwrap(), message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_rejects_a_non_object_message() {
    assert_eq!(
        skip_empty_text_blocks(&json("5")),
        Err(ContentError::MessageNotObject)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_rejects_a_null_text() {
    let message = json(r#"{"role":"assistant","content":[{"type":"text","text":null}]}"#);
    assert_eq!(
        skip_empty_text_blocks(&message),
        Err(ContentError::TextPartNotText { index: 0 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_skip_rejects_a_non_object_part() {
    let message = json(r#"{"role":"assistant","content":["bad"]}"#);
    assert_eq!(
        skip_empty_text_blocks(&message),
        Err(ContentError::PartNotObject { index: 0 })
    );
}

#[rstest::rstest]
#[case::absent(r#"{"role":"assistant"}"#)]
#[case::null(r#"{"role":"assistant","content":null}"#)]
#[case::string(r#"{"role":"assistant","content":"hi"}"#)]
#[case::object(r#"{"role":"assistant","content":{"a":1}}"#)]
fn bedrock_content_behavior_tests_behavior_process_keeps_non_list_content(
    #[case] message_json: &str,
) {
    let message = json(message_json);
    assert_eq!(process_empty_text_blocks(&message, None).unwrap(), message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_replaces_an_empty_list_with_continue() {
    let message = json(r#"{"role":"assistant","content":[]}"#);
    assert_eq!(
        process_empty_text_blocks(&message, None).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"Please continue."}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_replaces_all_empty_text_blocks_with_continue() {
    let message = json(
        r#"{"role":"assistant","content":[{"type":"text","text":" "},{"type":"text","text":""},{"type":"text","text":"\u001c"}]}"#,
    );
    assert_eq!(
        process_empty_text_blocks(&message, None).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"Please continue."}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_keeps_non_empty_parts() {
    let message = json(
        r#"{"role":"assistant","content":[{"type":"text","text":""},{"type":"text","text":"keep"}]}"#,
    );
    assert_eq!(
        process_empty_text_blocks(&message, None).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"keep"}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_never_reads_text_of_a_non_text_part() {
    let message =
        json(r#"{"role":"assistant","content":[{"type":"image_url","image_url":{"url":"x"}}]}"#);
    assert_eq!(process_empty_text_blocks(&message, None).unwrap(), message);
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_uses_a_custom_string_continue() {
    let message = json(r#"{"role":"assistant","content":[]}"#);
    assert_eq!(
        process_empty_text_blocks(&message, Some(&json(r#""carry on""#))).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"carry on"}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_uses_a_custom_dict_continue() {
    let message = json(r#"{"role":"assistant","content":[]}"#);
    let continue_message =
        json(r#"{"role":"assistant","content":[{"type":"text","text":"joined"}]}"#);
    assert_eq!(
        process_empty_text_blocks(&message, Some(&continue_message)).unwrap(),
        json(r#"{"role":"assistant","content":[{"type":"text","text":"joined"}]}"#)
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_rejects_a_part_without_type() {
    let message = json(r#"{"role":"assistant","content":[{"text":"x"}]}"#);
    assert_eq!(
        process_empty_text_blocks(&message, None),
        Err(ContentError::PartWithoutType { index: 0 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_rejects_a_text_part_without_text() {
    let message = json(r#"{"role":"assistant","content":[{"type":"text"}]}"#);
    assert_eq!(
        process_empty_text_blocks(&message, None),
        Err(ContentError::TextPartWithoutText { index: 0 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_rejects_a_null_text() {
    let message = json(r#"{"role":"assistant","content":[{"type":"text","text":null}]}"#);
    assert_eq!(
        process_empty_text_blocks(&message, None),
        Err(ContentError::TextPartNotText { index: 0 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_process_rejects_a_non_object_part() {
    let message = json(r#"{"role":"assistant","content":["bad"]}"#);
    assert_eq!(
        process_empty_text_blocks(&message, None),
        Err(ContentError::PartNotObject { index: 0 })
    );
}

#[test]
fn bedrock_content_behavior_tests_behavior_continue_text_keeps_the_message_fields() {
    let message = json(r#"{"role":"assistant","name":"bob","content":[]}"#);
    let processed = process_empty_text_blocks(&message, None).unwrap();
    assert_eq!(field(&processed, "name"), &json(r#""bob""#));
}
