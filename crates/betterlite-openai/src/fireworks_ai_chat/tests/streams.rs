use serde_json::Value;

use super::super::chunk::{FireworksChunk, chunk_parser};
use super::{fixed_clock, fixed_id};

fn chunk(text: &str) -> FireworksChunk {
    let value: Value = serde_json::from_str(text).expect("chunk JSON");
    chunk_parser(&value, fixed_id, fixed_clock).expect("chunk")
}

#[test]
fn chunk_parser_adds_the_fireworks_fields() {
    let parsed = chunk(
        r#"{
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "created": 1700000000,
        "model": "llama-v3",
        "perf_metrics": {"request_id": "r1"},
        "choices": [{"index": 0, "delta": {"content": "hi"}, "raw_output": {"seen": 1}}]
    }"#,
    );
    assert_eq!(parsed.reply.id, "chatcmpl-1");
    assert_eq!(parsed.reply.created, 1_700_000_000);
    assert_eq!(parsed.reply.choices[0].delta.content.as_deref(), Some("hi"));
    let fields = parsed.provider_specific_fields.expect("provider fields");
    assert!(fields.contains_key("fireworks_perf_metrics"));
    assert!(fields.contains_key("fireworks_raw_outputs"));
}

#[test]
fn chunk_parser_without_fireworks_fields_leaves_them_empty() {
    let parsed = chunk(
        r#"{
        "id": "chatcmpl-1",
        "created": 1700000000,
        "model": "llama-v3",
        "choices": [{"index": 0, "delta": {"content": "hi"}}]
    }"#,
    );
    assert!(parsed.provider_specific_fields.is_none());
}

#[test]
fn chunk_parser_defaults_the_id_and_object() {
    let parsed = chunk(r#"{"choices": []}"#);
    assert_eq!(parsed.reply.id, format!("chatcmpl-{}", fixed_id()));
    assert_eq!(parsed.reply.object, "chat.completion.chunk");
    assert_eq!(parsed.reply.created, 1_700_000_000);
}

#[test]
fn chunk_parser_keeps_the_openai_chunk_finish_reason() {
    let parsed = chunk(
        r#"{
        "id": "chatcmpl-1",
        "created": 1700000000,
        "choices": [{"index": 0, "delta": {}, "finish_reason": "max_tokens"}]
    }"#,
    );
    assert_eq!(
        parsed.reply.choices[0].finish_reason.as_deref(),
        Some("length")
    );
}
