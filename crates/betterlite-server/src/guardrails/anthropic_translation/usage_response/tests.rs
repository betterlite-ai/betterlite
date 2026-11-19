#![cfg(test)]
#![expect(
    clippy::default_numeric_fallback,
    clippy::disallowed_types,
    reason = "the tests compare provider JSON with serde_json values and plain number literals"
)]

use std::collections::BTreeMap;

use super::build_streaming_usage_response;
use crate::guardrails::anthropic_translation::sse_events::SseChunk;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::Value;

const START: &str = "event: message_start\ndata: {\"message\": {\"id\": \"msg_1\", \"model\": \"claude-test\", \"usage\": {\"input_tokens\": 10, \"output_tokens\": 1}}, \"type\": \"message_start\"}\n\n";
const TEXT: &str = "event: content_block_delta\ndata: {\"delta\": {\"text\": \"hi\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\n";
const DELTA: &str = "event: message_delta\ndata: {\"delta\": {\"stop_reason\": \"end_turn\"}, \"type\": \"message_delta\", \"usage\": {\"output_tokens\": 9}}\n\n";

fn chunks(parts: &[&str]) -> Vec<SseChunk> {
    vec![SseChunk::Bytes(parts.concat().into_bytes())]
}

fn request(value: Option<Value>) -> BTreeMap<String, GuardrailValue> {
    let mut data = BTreeMap::new();
    if let Some(model) = value {
        data.insert("model".to_owned(), serde_json::from_value(model).unwrap());
    }
    data
}

fn answer(chunks: &[SseChunk], request_data: Option<&BTreeMap<String, GuardrailValue>>) -> Value {
    serde_json::to_value(build_streaming_usage_response(chunks, request_data)).unwrap()
}

#[test]
fn usage_response_returns_none_without_a_bytes_chunk() {
    let event = SseChunk::Event(
        serde_json::from_str::<Value>("{\"type\": \"message_start\"}")
            .unwrap()
            .as_object()
            .unwrap()
            .clone(),
    );
    assert_eq!(
        build_streaming_usage_response(&[event, SseChunk::Other], None),
        None
    );
}

#[test]
fn usage_response_returns_the_usage_only_response() {
    let stream = chunks(&[START, TEXT, DELTA]);
    let data = request(Some(Value::String("claude-request".to_owned())));
    let value = answer(&stream, Some(&data));
    assert_eq!(value["model"], "claude-request");
    assert_eq!(value["object"], "chat.completion");
    assert_eq!(value["choices"][0]["finish_reason"], "stop");
    assert_eq!(value["choices"][0]["message"]["content"], "");
    assert_eq!(value["choices"][0]["message"]["role"], "assistant");
    assert_eq!(value["usage"]["prompt_tokens"], 10);
    assert_eq!(value["usage"]["completion_tokens"], 9);
    assert_eq!(value["usage"]["total_tokens"], 19);
    let created = value["created"].as_i64().unwrap();
    assert!(created > 1_600_000_000);
}

#[test]
fn usage_response_falls_back_to_the_streamed_model() {
    let stream = chunks(&[START, DELTA]);
    assert_eq!(answer(&stream, None)["model"], "claude-test");
}

#[test]
fn usage_response_returns_none_when_no_usage_event_is_found() {
    let stream = chunks(&[TEXT]);
    assert_eq!(build_streaming_usage_response(&stream, None), None);
}

#[test]
fn usage_response_reads_the_stop_reason() {
    let stream = chunks(&[
        START,
        "event: message_delta\ndata: {\"delta\": {\"stop_reason\": \"tool_use\"}, \"type\": \"message_delta\", \"usage\": {\"output_tokens\": 3}}\n\n",
    ]);
    assert_eq!(
        answer(&stream, None)["choices"][0]["finish_reason"],
        "tool_calls"
    );
}

#[test]
fn usage_response_ignores_event_and_other_chunks() {
    let event = SseChunk::Event(
        serde_json::from_str::<Value>(
            "{\"delta\": {\"stop_reason\": \"end_turn\"}, \"type\": \"message_delta\", \"usage\": {\"output_tokens\": 9}}",
        )
        .unwrap()
        .as_object()
        .unwrap()
        .clone(),
    );
    let stream = [
        SseChunk::Bytes(START.as_bytes().to_vec()),
        event,
        SseChunk::Other,
    ];
    let value = answer(&stream, None);
    assert_eq!(value["usage"]["completion_tokens"], 1);
}

#[test]
fn usage_response_treats_a_non_text_model_as_empty() {
    let stream = chunks(&[START, DELTA]);
    let data = request(Some(Value::from(5)));
    assert_eq!(answer(&stream, Some(&data))["model"], "claude-test");
}
