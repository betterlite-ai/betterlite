#![cfg(test)]
use super::*;
use serde_json::json;

#[test]
fn iter_events_reads_json_objects_and_ignores_bad_data() {
    let events = iter_sse_events(&SseChunk::Bytes(
        b"data: {\"type\":\"content_block_start\",\"index\":3}\n\ndata: bad\n\n".to_vec(),
    ));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["index"], 3);
}

#[test]
fn block_state_tracks_highest_open_and_seen() {
    let items = [
        SseChunk::Event(
            serde_json::from_value(json!({"type":"content_block_start","index":2})).unwrap(),
        ),
        SseChunk::Event(
            serde_json::from_value(json!({"type":"content_block_start","index":5})).unwrap(),
        ),
        SseChunk::Event(
            serde_json::from_value(json!({"type":"content_block_stop","index":5})).unwrap(),
        ),
    ];
    assert_eq!(content_block_state(&items), (Some(2), Some(5)));
}

#[test]
fn text_event_line_drives_text_extraction() {
    assert_eq!(extract_text_from_sse(b"event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n"), "hi");
}

#[test]
fn has_ended_requires_non_null_stop_reason() {
    let ended = SseChunk::Event(
        serde_json::from_value(json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}))
            .unwrap(),
    );
    assert!(streaming_has_ended(&[ended]));
}

#[test]
fn scan_key_includes_text_and_waits_for_end_before_tools() {
    let item = SseChunk::Bytes(b"event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"a\"}}\n\n".to_vec());
    let key = streaming_scan_key(&[item]);
    assert_eq!(key.texts, vec!["a"]);
    assert!(!key.stream_ended);
}

#[test]
fn fingerprints_recognize_tool_use_json_type() {
    let event = json!({"type":"content_block_start","content_block":{"type":"tool_use","id":"x"}});
    let chunk = SseChunk::Event(event.as_object().unwrap().clone());
    assert_eq!(streamed_tool_use_fingerprints(&[chunk]).len(), 1);
}

#[test]
fn replays_the_anthropic_sse_events_corpus() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_sse_events");
    for entry in std::fs::read_dir(directory).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let chunk = SseChunk::Bytes(bytes);
        let _ = iter_sse_events(&chunk);
        let _ = content_block_state(&[chunk]);
    }
}
