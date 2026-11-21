#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use serde_json::{Value, json};

use super::error::PostCallHookError;
use super::post_call_hook_response;

fn value(raw: Value) -> GuardrailValue {
    serde_json::from_value(raw).expect("a GuardrailValue")
}

fn to_json(item: &GuardrailValue) -> Value {
    serde_json::to_value(item).expect("a JSON value")
}

fn model_response() -> GuardrailValue {
    value(json!({
        "id": "chatcmpl-1",
        "created": 1,
        "model": "gpt-4o",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hello"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
    }))
}

#[test]
fn hook_translates_a_model_response_into_an_anthropic_message() {
    let translated = to_json(&post_call_hook_response(&model_response()).unwrap());
    assert_eq!(translated["id"].as_str(), Some("chatcmpl-1"));
    assert_eq!(translated["type"].as_str(), Some("message"));
    assert_eq!(translated["role"].as_str(), Some("assistant"));
    assert_eq!(translated["model"].as_str(), Some("gpt-4o"));
    assert_eq!(translated["stop_reason"].as_str(), Some("end_turn"));
    assert_eq!(translated["usage"]["input_tokens"].as_u64(), Some(3));
    assert_eq!(translated["usage"]["output_tokens"].as_u64(), Some(2));
    assert_eq!(translated["content"][0]["type"].as_str(), Some("text"));
    assert_eq!(translated["content"][0]["text"].as_str(), Some("hello"));
}

#[test]
fn hook_translates_a_response_without_usage_to_zero_counters() {
    let response = value(json!({
        "id": "chatcmpl-2",
        "model": "",
        "choices": [{"message": {"role": "assistant", "content": "hi"}}]
    }));
    let translated = to_json(&post_call_hook_response(&response).unwrap());
    assert_eq!(translated["model"].as_str(), Some("unknown-model"));
    assert_eq!(translated["usage"]["input_tokens"].as_u64(), Some(0));
    assert_eq!(translated["usage"]["output_tokens"].as_u64(), Some(0));
}

#[test]
fn hook_returns_an_anthropic_message_unchanged() {
    let response = value(json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "hi"}]
    }));
    assert_eq!(post_call_hook_response(&response).unwrap(), response);
}

#[rstest]
#[case(json!("plain"))]
#[case(json!([1, 2]))]
#[case(json!(null))]
#[case(json!(7))]
fn hook_returns_non_objects_unchanged(#[case] raw: Value) {
    let response = value(raw);
    assert_eq!(post_call_hook_response(&response).unwrap(), response);
}

#[rstest]
#[case(json!({"choices": []}))]
#[case(json!({"choices": [1]}))]
fn hook_rejects_choices_that_do_not_parse_as_a_model_response(#[case] raw: Value) {
    let error = post_call_hook_response(&value(raw)).unwrap_err();
    assert!(matches!(error, PostCallHookError::Response { .. }));
}
