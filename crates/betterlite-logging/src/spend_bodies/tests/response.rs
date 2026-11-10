use super::support::{capture_logs, mapping, policy, redacting, value};
use crate::spend_bodies::response::{ResponseKwargs, storage_response};
use crate::spend_bodies::support::NodeAllocator;
use crate::spend_sanitize::value::SpendMapping;

fn stored(payload: &SpendMapping, kwargs: Option<&ResponseKwargs<'_>>, redact: bool) -> String {
    let policy = if redact {
        redacting(policy(true, 2048))
    } else {
        policy(true, 2048)
    };
    storage_response(Some(payload), kwargs, policy, &mut NodeAllocator::default())
}

#[test]
fn disabled_storage_returns_an_empty_object() {
    let payload = mapping(r#"{"response": "hello"}"#);
    assert_eq!(
        storage_response(
            Some(&payload),
            None,
            policy(false, 2048),
            &mut NodeAllocator::default()
        ),
        "{}"
    );
}

#[test]
fn a_missing_or_null_response_returns_an_empty_object() {
    assert_eq!(stored(&mapping("{}"), None, false), "{}");
    assert_eq!(stored(&mapping(r#"{"response": null}"#), None, false), "{}");
    assert_eq!(
        storage_response(
            None,
            None,
            policy(true, 2048),
            &mut NodeAllocator::default()
        ),
        "{}"
    );
}

#[test]
fn a_string_response_loses_nul_bytes_without_json_quoting() {
    let payload = mapping(r#"{"response": "abc\u0000def"}"#);
    assert_eq!(stored(&payload, None, false), "abcdef");
}

#[test]
fn a_mapping_response_is_serialized_as_json() {
    let payload = mapping(r#"{"response": {"choices": [{"message": {"content": "hi"}}]}}"#);
    assert_eq!(
        stored(&payload, None, false),
        r#"{"choices": [{"message": {"content": "hi"}}]}"#
    );
}

#[test]
fn redaction_rewrites_choice_content_and_tool_arguments() {
    let payload = mapping(
        r#"{"response": {"choices": [{"message": {"content": "hi", "thinking_blocks": [1], "audio": {"id": "a"}, "tool_calls": [{"function": {"name": "f", "arguments": "{}"}}], "function_call": {"arguments": "{}"}}}]}}"#,
    );
    assert_eq!(
        stored(&payload, None, true),
        r#"{"choices": [{"message": {"content": "redacted-by-litellm", "thinking_blocks": null, "audio": null, "tool_calls": [{"function": {"name": "f", "arguments": "redacted-by-litellm"}}], "function_call": {"arguments": "redacted-by-litellm"}}}]}"#
    );
}

#[test]
fn redaction_turns_a_non_message_response_into_a_text_object() {
    let payload = mapping(r#"{"response": ["hi"]}"#);
    assert_eq!(
        stored(&payload, None, true),
        r#"{"text": "redacted-by-litellm"}"#
    );
}

#[test]
fn redaction_rewrites_responses_output_items() {
    let payload = mapping(
        r#"{"response": {"output": [{"type": "function_call", "arguments": "{}"}, {"type": "custom_tool_call", "input": "x"}, {"type": "message", "content": [{"text": "hi", "refusal": "no"}]}]}}"#,
    );
    assert_eq!(
        stored(&payload, None, true),
        r#"{"output": [{"type": "function_call", "arguments": "redacted-by-litellm"}, {"type": "custom_tool_call", "input": "redacted-by-litellm"}, {"type": "message", "content": [{"text": "redacted-by-litellm", "refusal": "redacted-by-litellm"}]}]}"#
    );
}

#[test]
fn realtime_tool_calls_overlay_mapping_responses_only() {
    let payload = mapping(r#"{"response": {"id": "x"}}"#);
    let tool_calls = value(r#"[{"id": "call"}]"#);
    let kwargs = ResponseKwargs {
        realtime_tool_calls: Some(&tool_calls),
        litellm_params: None,
        dynamic_params: None,
    };
    assert_eq!(
        stored(&payload, Some(&kwargs), false),
        r#"{"id": "x", "tool_calls": [{"id": "call"}]}"#
    );
    let string_payload = mapping(r#"{"response": "plain"}"#);
    assert_eq!(stored(&string_payload, Some(&kwargs), false), "plain");
}

#[test]
fn truncates_a_long_string_response_and_logs_the_safeguard_note() {
    let payload = mapping(r#"{"response": "abcdefghijklmnop"}"#);
    let (logs, guard) = capture_logs();
    let stored = storage_response(
        Some(&payload),
        None,
        policy(true, 10),
        &mut NodeAllocator::default(),
    );
    drop(guard);
    assert!(stored.contains("litellm_truncated skipped"));
    assert!(logs.text().contains("response was truncated"));
}
