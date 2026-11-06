#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{ClaudeMappingError, map};
use crate::helicone::test_support::json;

fn completion(message: &str, finish_reason: &str) -> Json {
    json(&format!(
        r#"{{"id": "chatcmpl-9", "choices": [{{"finish_reason": "{finish_reason}", "message": {message}}}], "usage": {{"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}}}}"#
    ))
}

#[rstest]
#[case::text(
    r#"{"role": "assistant", "content": "hello"}"#,
    "stop",
    r#"[{"type": "text", "text": "hello"}]"#
)]
#[case::function_tool_call(
    r#"{"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\":1}"}}]}"#,
    "tool_calls",
    r#"[{"type": "tool_use", "id": "call_1", "name": "lookup", "input": "{\"q\":1}"}]"#
)]
#[case::custom_tool_call(
    r#"{"role": "assistant", "tool_calls": [{"id": "call_2", "type": "custom", "custom": {"name": "patch", "input": "raw text"}}]}"#,
    "tool_calls",
    r#"[{"type": "tool_use", "id": "call_2", "name": "patch", "input": "raw text"}]"#
)]
#[case::tool_call_without_function_or_custom_is_skipped(
    r#"{"role": "assistant", "tool_calls": [{"id": "call_3", "type": "function"}, {"id": "call_4", "type": "function", "function": {"name": "keep", "arguments": "{}"}}]}"#,
    "tool_calls",
    r#"[{"type": "tool_use", "id": "call_4", "name": "keep", "input": "{}"}]"#
)]
#[case::tool_calls_win_over_text(
    r#"{"role": "assistant", "content": "ignored", "tool_calls": [{"id": "call_5", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}"#,
    "tool_calls",
    r#"[{"type": "tool_use", "id": "call_5", "name": "f", "input": "{}"}]"#
)]
#[case::empty_tool_calls_fall_back_to_text(
    r#"{"role": "assistant", "content": "plain", "tool_calls": []}"#,
    "stop",
    r#"[{"type": "text", "text": "plain"}]"#
)]
#[case::empty_content_gives_no_blocks(r#"{"role": "assistant", "content": ""}"#, "stop", "[]")]
#[case::null_content_gives_no_blocks(r#"{"role": "assistant", "content": null}"#, "length", "[]")]
fn claude_mapping_builds_the_content_blocks(
    #[case] message: &str,
    #[case] finish_reason: &str,
    #[case] content: &str,
) {
    let mapped = map("claude-3-opus", &completion(message, finish_reason)).unwrap();
    let expected = json(&format!(
        r#"{{"id": "chatcmpl-9", "type": "message", "role": "assistant", "model": "claude-3-opus", "content": {content}, "stop_reason": "{finish_reason}", "stop_sequence": null, "usage": {{"input_tokens": 11, "output_tokens": 7}}}}"#
    ));
    assert_eq!(json(&serde_json::to_string(&mapped).unwrap()), expected);
}

#[test]
fn claude_mapping_keeps_a_missing_finish_reason_null() {
    let response = json(
        r#"{"id": "x", "choices": [{"message": {"content": "hi"}}], "usage": {"prompt_tokens": 1, "completion_tokens": 2}}"#,
    );
    assert_eq!(map("claude-3", &response).unwrap().stop_reason, None);
}

#[test]
fn claude_mapping_does_not_send_a_prompt_string() {
    let mapped = map("claude-3", &completion(r#"{"content": "hi"}"#, "stop")).unwrap();
    let text = serde_json::to_string(&mapped).unwrap();
    assert!(!text.contains("Human:") && !text.contains("prompt"));
}

#[test]
fn claude_mapping_rejects_a_response_without_choices() {
    let response = json(
        r#"{"id": "x", "choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": 2}}"#,
    );
    assert!(matches!(
        map("claude-3", &response),
        Err(ClaudeMappingError::NoChoices)
    ));
}

#[test]
fn claude_mapping_rejects_a_response_that_is_not_a_completion() {
    let response = json(r#"{"error": "boom"}"#);
    assert!(matches!(
        map("claude-3", &response),
        Err(ClaudeMappingError::Response(_))
    ));
}
