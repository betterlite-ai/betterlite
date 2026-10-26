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
fn preserves_original_and_shallow_message_fields() {
    let messages = vec![json!({"role":"user","content":[{"type":"text","text":"hi"}],"name":"n"})];
    let (normalized, original) = normalize_messages(&messages, "completion").unwrap();
    assert_eq!(normalized[0]["content"], "hi");
    assert_eq!(normalized[0]["name"], "n");
    assert_eq!(original, messages);
}
