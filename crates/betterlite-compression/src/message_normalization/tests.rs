#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn extracts_nested_tool_result_text_and_ignores_media() {
    let content = json!([{"type":"image"},{"type":"tool_result","content":[{"type":"text","text":"one"},{"type":"thinking","thinking":"no"}]}]);
    assert_eq!(content_to_text(&content), "one");
}

#[test]
fn rejects_unknown_call_type() {
    let error = normalize_messages(&[], "chat").unwrap_err();
    assert!(matches!(
        &error,
        CompressionError::UnsupportedCallType { call_type, .. } if call_type == "chat"
    ));
    assert_eq!(
        error.to_string(),
        "Unsupported call_type='chat' for compression. Expected one of: ['acompletion', 'anthropic_messages', 'completion']."
    );
}

#[test]
fn parses_supported_call_types() {
    assert_eq!(
        normalize_messages(&[], "completion").unwrap().0,
        CallType::Completion
    );
    assert_eq!(
        normalize_messages(&[], "acompletion").unwrap().0,
        CallType::Acompletion
    );
    assert_eq!(
        normalize_messages(&[], "anthropic_messages").unwrap().0,
        CallType::AnthropicMessages
    );
}

#[test]
fn finds_latest_user_message() {
    let messages = vec![
        json!({"role":"user","content":"old"}),
        json!({"role":"user","content":[{"type":"text","text":"new"}]}),
        json!({"role":"assistant","content":"answer"}),
    ];
    assert_eq!(extract_last_user_message(&messages), "new");
    let without_user = vec![json!({"role":"assistant","content":"answer"})];
    assert_eq!(extract_last_user_message(&without_user), "");
}

#[test]
fn preserves_original_and_shallow_message_fields() {
    let messages = vec![json!({"role":"user","content":[{"type":"text","text":"hi"}],"name":"n"})];
    let (_, normalized, original) = normalize_messages(&messages, "completion").unwrap();
    assert_eq!(normalized[0]["content"], "hi");
    assert_eq!(normalized[0]["name"], "n");
    assert_eq!(original, messages);
}
