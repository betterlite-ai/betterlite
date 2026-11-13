#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{
    ExtractedInput, InputWriteBackTarget, MessageTextTarget, ScannedText, ScannedToolCall,
    ToolUseInputTarget, is_client_tool_use,
};

fn block_map(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn target_message_content_holds_the_message_index() {
    let target = MessageTextTarget::MessageContent { msg_idx: 2 };
    assert_eq!(target, MessageTextTarget::MessageContent { msg_idx: 2 });
}

#[rstest]
#[case::content_block(MessageTextTarget::ContentBlockText { msg_idx: 1, content_idx: 2 })]
#[case::tool_result_string(MessageTextTarget::ToolResultString { msg_idx: 1, content_idx: 2 })]
#[case::tool_result_block(MessageTextTarget::ToolResultBlockText {
    msg_idx: 1,
    content_idx: 2,
    block_idx: 3,
})]
fn target_message_variants_keep_their_indexes(#[case] target: MessageTextTarget) {
    assert_eq!(target, target.clone());
    assert_eq!(
        InputWriteBackTarget::Message(target.clone()),
        InputWriteBackTarget::Message(target)
    );
}

#[test]
fn target_system_variants_keep_their_indexes() {
    assert_eq!(
        InputWriteBackTarget::SystemBlockText { block_idx: 3 },
        InputWriteBackTarget::SystemBlockText { block_idx: 3 }
    );
    assert_eq!(
        InputWriteBackTarget::SystemString,
        InputWriteBackTarget::SystemString
    );
}

#[test]
fn target_tool_use_input_holds_the_message_and_block_indexes() {
    assert_eq!(
        ToolUseInputTarget {
            msg_idx: 1,
            content_idx: 2
        },
        ToolUseInputTarget {
            msg_idx: 1,
            content_idx: 2
        }
    );
}

#[test]
fn extracted_input_empty_has_no_entries() {
    let empty = ExtractedInput::empty();
    assert!(empty.scanned.is_empty());
    assert!(empty.images.is_empty());
    assert!(empty.tool_calls.is_empty());
}

#[test]
fn extracted_input_keeps_scanned_texts_and_tool_calls() {
    let extracted = ExtractedInput {
        scanned: vec![ScannedText {
            text: "hello".to_owned(),
            target: InputWriteBackTarget::Message(MessageTextTarget::MessageContent { msg_idx: 0 }),
        }],
        images: vec!["https://a.test/i".to_owned()],
        tool_calls: vec![ScannedToolCall {
            tool_call: json!({"id": "t1", "type": "function", "function": {"name": "f", "arguments": "{}"}}),
            target: ToolUseInputTarget {
                msg_idx: 0,
                content_idx: 1,
            },
        }],
    };
    assert_eq!(extracted.scanned.len(), 1);
    assert_eq!(extracted.images, ["https://a.test/i"]);
    assert_eq!(extracted.tool_calls.len(), 1);
}

#[rstest]
#[case::tool_use_with_object_input(
    json!({"type": "tool_use", "id": "t1", "name": "f", "input": {"q": "x"}}),
    true
)]
#[case::empty_object_input(json!({"type": "tool_use", "id": "t1", "name": "f", "input": {}}), true)]
#[case::string_input(json!({"type": "tool_use", "id": "t1", "name": "f", "input": "x"}), false)]
#[case::null_input(json!({"type": "tool_use", "id": "t1", "name": "f", "input": null}), false)]
#[case::missing_input(json!({"type": "tool_use", "id": "t1", "name": "f"}), false)]
#[case::missing_id(json!({"type": "tool_use", "name": "f", "input": {}}), false)]
#[case::number_id(json!({"type": "tool_use", "id": 1, "name": "f", "input": {}}), false)]
#[case::missing_name(json!({"type": "tool_use", "id": "t1", "input": {}}), false)]
#[case::number_name(json!({"type": "tool_use", "id": "t1", "name": 2, "input": {}}), false)]
#[case::text_block(json!({"type": "text", "text": "x"}), false)]
#[case::other_type(json!({"type": "tool_result", "id": "t1", "name": "f", "input": {}}), false)]
#[case::missing_type(json!({"id": "t1", "name": "f", "input": {}}), false)]
#[case::non_string_type(json!({"type": 1, "id": "t1", "name": "f", "input": {}}), false)]
fn client_tool_use_requires_the_declared_fields(#[case] block: Value, #[case] expected: bool) {
    assert_eq!(is_client_tool_use(&block_map(&block)), expected);
}
