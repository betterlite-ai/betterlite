#![cfg(test)]

use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::transforms::{transform_choices, transform_messages, transform_responses_output};

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn json(message: &OtelModelValue) -> String {
    serde_json::to_string(message).expect("json")
}

fn json_all(messages: &[OtelModelValue]) -> Vec<String> {
    messages.iter().map(json).collect()
}

#[test]
fn string_messages_become_one_system_message() {
    let transformed = transform_messages(&OtelModelValue::String("be brief".to_owned()));
    assert_eq!(
        json_all(&transformed),
        json_all(&[value(
            r#"{"role": "system", "parts": [{"type": "text", "content": "be brief"}]}"#
        )])
    );
}

#[test]
fn list_messages_become_role_parts_pairs() {
    let messages = value(
        r#"[{"role": "assistant", "content": "hello", "id": "m-1", "tool_calls": [{"id": "t1"}], "tool_call_id": "t1"}]"#,
    );
    assert_eq!(
        json_all(&transform_messages(&messages)),
        json_all(&[value(
            r#"{
                "role": "assistant",
                "parts": [{"type": "text", "content": "hello"}],
                "id": "m-1",
                "tool_calls": [{"id": "t1"}],
                "tool_call_id": "t1"
            }"#
        )])
    );
}

#[test]
fn multimodal_parts_pass_through_and_scalars_become_text() {
    let messages = value(
        r#"[{"role": "user", "content": [{"type": "text", "text": "describe"}, "bare", 7]}]"#,
    );
    assert_eq!(
        json_all(&transform_messages(&messages)),
        json_all(&[value(
            r#"{
                "role": "user",
                "parts": [
                    {"type": "text", "text": "describe"},
                    {"type": "text", "content": "bare"},
                    {"type": "text", "content": "7"}
                ]
            }"#
        )])
    );
}

#[test]
fn missing_role_and_content_take_the_litellm_defaults() {
    let messages =
        value(r#"[{"content": "text"}, {"role": "user"}, {"role": "user", "content": null}]"#);
    assert_eq!(
        json_all(&transform_messages(&messages)),
        json_all(&[
            value(r#"{"role": "user", "parts": [{"type": "text", "content": "text"}]}"#),
            value(r#"{"role": "user", "parts": [{"type": "text", "content": ""}]}"#),
            value(r#"{"role": "user", "parts": []}"#),
        ])
    );
}

#[test]
fn choices_carry_the_finish_reason() {
    let choices = value(
        r#"[{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}, {"finish_reason": "length"}]"#,
    );
    let OtelModelValue::Array(items) = choices else {
        panic!("choices must be a list");
    };
    assert_eq!(
        json_all(&transform_choices(&items)),
        json_all(&[
            value(
                r#"{"role": "assistant", "parts": [{"type": "text", "content": "hi"}], "finish_reason": "stop"}"#
            ),
            value(
                r#"{"role": "user", "parts": [{"type": "text", "content": ""}], "finish_reason": "length"}"#
            ),
        ])
    );
}

#[test]
fn responses_output_messages_and_function_calls_are_transformed() {
    let output = value(
        r#"[
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}, {"type": "refusal", "refusal": "no"}]},
            {"type": "function_call", "name": "get_weather", "arguments": "{}", "call_id": "call-7"},
            {"type": "message", "role": "assistant", "content": [{"type": "refusal", "refusal": "no"}]},
            {"type": "reasoning"}
        ]"#,
    );
    let OtelModelValue::Array(items) = output else {
        panic!("output must be a list");
    };
    assert_eq!(
        json_all(&transform_responses_output(&items)),
        json_all(&[
            value(r#"{"role": "assistant", "parts": [{"type": "text", "content": "hello"}]}"#),
            value(
                r#"{"role": "assistant", "parts": [{"type": "tool_call", "name": "get_weather", "arguments": "{}", "id": "call-7"}]}"#
            ),
        ])
    );
}

#[test]
fn responses_function_calls_without_a_name_or_call_id_stay_minimal() {
    let output = value(r#"[{"type": "function_call"}]"#);
    let OtelModelValue::Array(items) = output else {
        panic!("output must be a list");
    };
    assert_eq!(
        json_all(&transform_responses_output(&items)),
        json_all(&[value(
            r#"{"role": "assistant", "parts": [{"type": "tool_call", "name": "", "arguments": ""}]}"#
        )])
    );
}
