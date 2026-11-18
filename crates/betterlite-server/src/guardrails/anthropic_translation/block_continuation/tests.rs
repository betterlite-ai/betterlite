#![cfg(test)]
use super::*;

#[test]
fn standalone_block_is_fake_anthropic_stream() {
    let chunks = build_block_sse_chunks("blocked", "m", None, false, None, "msg_1").unwrap();
    assert!(
        chunks
            .first()
            .unwrap()
            .starts_with(b"event: message_start\n")
    );
}

#[test]
fn standalone_block_uses_original_usage() {
    let original = serde_json::json!({"usage":{"input_tokens":4,"output_tokens":1}});
    let chunks =
        build_block_sse_chunks("blocked", "m", Some(&original), false, None, "msg_1").unwrap();
    let start = std::str::from_utf8(&chunks[0]).unwrap();
    let message: serde_json::Value = serde_json::from_str(
        start
            .lines()
            .nth(1)
            .unwrap()
            .strip_prefix("data: ")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(message["message"]["usage"]["input_tokens"], 4);
    assert_eq!(message["message"]["usage"]["output_tokens"], 1);
}

#[test]
fn continuation_closes_previous_block_without_second_start() {
    let previous = [SseChunk::Event(
        serde_json::from_value(serde_json::json!({"type":"content_block_start","index":0}))
            .unwrap(),
    )];
    let chunks =
        build_block_sse_chunks("blocked", "m", None, true, Some(&previous), "msg_1").unwrap();
    assert!(
        chunks
            .iter()
            .all(|chunk| !chunk.starts_with(b"event: message_start\n"))
    );
    assert_eq!(chunks.len(), 6);
}

#[test]
fn error_frames_match_shared_error_builder() {
    assert!(build_stream_error_items("blocked").unwrap()[0].starts_with(b"event: error\n"));
}

#[test]
fn continuation_uses_python_json_separators_and_ascii_escapes() {
    let value = serde_json::json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {"type": "text_delta", "text": "a,b 🗝"}
    });
    assert_eq!(
        python_json(&value),
        r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "a,b \ud83d\udd5d"}}"#
    );
}

#[test]
fn continuation_keeps_python_field_order_for_nested_event_shapes() {
    let start = serde_json::json!({
        "type": "content_block_start",
        "index": 2,
        "content_block": {"type": "text", "text": ""}
    });
    assert_eq!(
        python_json(&start),
        r#"{"type": "content_block_start", "index": 2, "content_block": {"type": "text", "text": ""}}"#
    );
    let delta = serde_json::json!({
        "type": "message_delta",
        "delta": {"stop_reason": "end_turn", "stop_sequence": null},
        "usage": {"output_tokens": 0}
    });
    assert_eq!(
        python_json(&delta),
        r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 0}}"#
    );
}

#[test]
fn python_json_formats_arrays_with_default_separators() {
    let value = serde_json::json!([{"type":"text","text":"x"}, 1]);
    assert_eq!(python_json(&value), r#"[{"type": "text", "text": "x"}, 1]"#);
}
