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
    assert!(error.contains("completion"));
}

#[test]
fn normalizes_call_shapes_and_finds_latest_user() {
    assert_eq!(normalize_call_type("acompletion"), "acompletion");
    assert!(is_anthropic_call_type("anthropic_messages"));
    assert!(!is_anthropic_call_type("completion"));
    let messages = vec![
        json!({"role":"user","content":"old"}),
        json!({"role":"assistant","content":"answer"}),
        json!({"role":"user","content":[{"type":"text","text":"new"}]}),
    ];
    assert_eq!(extract_last_user_message(&messages), "new");
}

#[test]
fn preserves_original_and_shallow_message_fields() {
    let messages = vec![json!({"role":"user","content":[{"type":"text","text":"hi"}],"name":"n"})];
    let (normalized, original) = normalize_messages(&messages, "completion").unwrap();
    assert_eq!(normalized[0]["content"], "hi");
    assert_eq!(normalized[0]["name"], "n");
    assert_eq!(original, messages);
}
