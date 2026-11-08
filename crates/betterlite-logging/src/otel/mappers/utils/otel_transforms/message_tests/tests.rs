#![cfg(test)]

use crate::otel::mappers::utils::{
    message_content, output_messages, serialize_messages, stringify_message,
};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;

fn message(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("message")
}

fn call_with_choices() -> LLMCallSpanData {
    let payload: Payload = serde_json::from_str(
        r#"{
            "id": "call-1",
            "model": "gpt-4o",
            "response": {
                "choices": [
                    {"message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"},
                    {"finish_reason": "stop"}
                ]
            }
        }"#,
    )
    .expect("payload");
    LLMCallSpanData::from_standard_logging_payload(
        &payload,
        LLMCallOptions {
            capture_content: true,
            time_to_first_chunk_seconds: None,
            request_route: None,
            trace: TraceControls::default(),
            session_id: None,
        },
    )
    .expect("llm call")
}

#[test]
fn serialize_messages_keeps_multimodal_content_and_tool_calls() {
    let messages = [
        message(
            r#"{"role": "user", "content": [{"type": "text", "text": "hi"}, {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}}]}"#,
        ),
        message(
            r#"{"role": "assistant", "content": "hello", "tool_calls": [{"id": "t1", "type": "function", "function": {"name": "f", "arguments": "{}"}}], "tool_call_id": "t1"}"#,
        ),
    ];
    let serialized = serialize_messages(&messages).expect("serialized");
    let parsed: OtelModelValue = serde_json::from_str(&serialized).expect("json");
    let expected: OtelModelValue = serde_json::from_str(
        r#"[
            {"role": "user", "content": [{"type": "text", "text": "hi"}, {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}}]},
            {"role": "assistant", "content": "hello", "tool_calls": [{"id": "t1", "type": "function", "function": {"name": "f", "arguments": "{}"}}], "tool_call_id": "t1"}
        ]"#,
    )
    .expect("expected");
    assert_eq!(
        serde_json::to_string(&parsed).expect("json"),
        serde_json::to_string(&expected).expect("json")
    );
}

#[test]
fn serialize_messages_returns_none_without_messages() {
    assert_eq!(serialize_messages(&[]), None);
}

#[test]
fn stringify_message_encodes_one_message() {
    let serialized = stringify_message(&message(r#"{"role": "user"}"#));
    assert_eq!(serialized, Some(r#"{"role":"user"}"#.to_owned()));
}

#[test]
fn output_messages_reads_the_message_of_each_choice() {
    let call = call_with_choices();
    let messages = output_messages(&call);
    assert_eq!(messages.len(), 1);
    let extracted = messages.first().expect("message");
    assert_eq!(
        serde_json::to_string(extracted).expect("json"),
        serde_json::to_string(&message(r#"{"content": "hello", "role": "assistant"}"#))
            .expect("json")
    );
}

#[test]
fn message_content_joins_the_text_parts() {
    let content = message(
        r#"{"role": "user", "content": [{"type": "text", "text": "a"}, {"type": "image_url", "image_url": {"url": "u"}}, {"type": "text", "text": "b"}]}"#,
    );
    assert_eq!(message_content(&content), Some("ab".to_owned()));
    let plain = message(r#"{"role": "user", "content": "text"}"#);
    assert_eq!(message_content(&plain), Some("text".to_owned()));
}

#[test]
fn message_content_returns_none_without_text() {
    assert_eq!(
        message_content(&message(
            r#"{"role": "user", "content": [{"type": "image_url"}]}"#
        )),
        None
    );
    assert_eq!(
        message_content(&message(r#"{"role": "user", "content": ""}"#)),
        Some(String::new())
    );
    assert_eq!(message_content(&message(r#"{"role": "user"}"#)), None);
}
