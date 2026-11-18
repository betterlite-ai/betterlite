#![cfg(test)]
use super::{
    SseChunk, content_block_state, extract_text_from_sse, iter_sse_events,
    streamed_tool_use_fingerprints, streaming_has_ended, streaming_scan_key,
    streaming_string_so_far,
};
use serde_json::{Map, Value, json};
use std::{
    fs, io,
    path::Path,
    slice, str,
    sync::{Arc, Mutex},
};
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

#[test]
fn iter_events_reads_json_objects_and_ignores_bad_data() {
    let events = iter_sse_events(&SseChunk::Bytes(
        b"data: {\"type\":\"content_block_start\",\"index\":3}\n\ndata: bad\n\n".to_vec(),
    ));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["index"], 3);
}

#[test]
fn iter_events_preserves_parsed_event_and_ignores_other_item() {
    let event = json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}});
    let chunk = SseChunk::Event(event.as_object().unwrap().clone());
    assert_eq!(&iter_sse_events(&chunk)[0], event.as_object().unwrap());
    assert!(iter_sse_events(&SseChunk::Other).is_empty());
}

#[test]
fn block_state_tracks_highest_open_and_seen() {
    let items = [
        SseChunk::Event(
            serde_json::from_value(json!({"type":"content_block_start","index":2_i64})).unwrap(),
        ),
        SseChunk::Event(
            serde_json::from_value(json!({"type":"content_block_start","index":5_i64})).unwrap(),
        ),
        SseChunk::Event(
            serde_json::from_value(json!({"type":"content_block_stop","index":5_i64})).unwrap(),
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
fn has_ended_bytes_uses_event_line_and_requires_stop_reason() {
    let stopped = SseChunk::Bytes(
        b"event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n".to_vec(),
    );
    let null_stop = SseChunk::Bytes(
        b"event: message_delta\ndata: {\"delta\":{\"stop_reason\":null}}\n\n".to_vec(),
    );
    assert!(streaming_has_ended(&[null_stop, stopped]));
}

#[test]
fn has_ended_bytes_requires_a_non_null_stop_reason() {
    let null_stop = SseChunk::Bytes(
        b"event: message_delta\ndata: {\"delta\":{\"stop_reason\":null}}\n\n".to_vec(),
    );
    assert!(!streaming_has_ended(&[null_stop]));
}

#[test]
fn event_text_delta_is_read_without_checking_its_event_type() {
    let event = serde_json::from_value(json!({
        "type":"other_event",
        "delta":{"type":"text_delta","text":"seen"}
    }))
    .unwrap();
    assert_eq!(streaming_string_so_far(&[SseChunk::Event(event)]), "seen");
}

#[test]
fn mixed_chunks_append_text_in_order_and_skip_non_text() {
    let items = [
        SseChunk::Bytes(b"event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"first\"}}\n\n".to_vec()),
        SseChunk::Other,
        SseChunk::Event(serde_json::from_value(json!({"delta":{"type":"input_json_delta","partial_json":"ignored"}})).unwrap()),
        SseChunk::Event(serde_json::from_value(json!({"delta":{"type":"text_delta","text":" second"}})).unwrap()),
    ];
    assert_eq!(streaming_string_so_far(&items), "first second");
}

#[test]
fn invalid_utf8_does_not_end_stream_and_text_reader_returns_empty() {
    let invalid = SseChunk::Bytes(vec![0xff]);
    assert!(!streaming_has_ended(slice::from_ref(&invalid)));
    assert_eq!(extract_text_from_sse(&[0xff]), "");
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
    assert_eq!(
        streamed_tool_use_fingerprints(&[chunk]),
        vec![r#"{"id":"x","type":"tool_use"}"#]
    );
}

#[test]
fn scan_key_releases_tool_fingerprint_only_after_end() {
    let start = SseChunk::Bytes(
        b"data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\"}}\n\n".to_vec(),
    );
    let end = SseChunk::Bytes(
        b"event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n".to_vec(),
    );
    let pending = streaming_scan_key(slice::from_ref(&start));
    assert!(pending.tool_calls.is_empty());
    assert!(pending.tool_calls_in_flight);
    let ended = streaming_scan_key(&[start, end]);
    assert_eq!(ended.tool_calls.len(), 1);
    assert!(ended.stream_ended);
    assert!(!ended.tool_calls_in_flight);
}

#[test]
fn replays_the_anthropic_sse_events_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_sse_events");
    for entry in fs::read_dir(directory).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        let chunk = SseChunk::Bytes(bytes);
        let _events = iter_sse_events(&chunk);
        let _state = content_block_state(slice::from_ref(&chunk));
        let _text = streaming_string_so_far(slice::from_ref(&chunk));
        let _ended = streaming_has_ended(&[chunk]);
    }
}

#[test]
fn iter_events_replaces_invalid_utf8_and_keeps_the_event() {
    let mut bytes = b"data: {\"type\":\"content_block_start\",\"index\":1,\"text\":\"".to_vec();
    bytes.push(0xff);
    bytes.extend_from_slice(b"\"}\n\n");
    let events = iter_sse_events(&SseChunk::Bytes(bytes));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["text"], "\u{fffd}");
}

#[test]
fn iter_events_replaces_each_invalid_sequence_once() {
    let events = iter_sse_events(&SseChunk::Bytes(b"data: {\"text\":\"a\xffb\"}".to_vec()));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["text"], "a\u{fffd}b");
}

#[test]
fn iter_events_ignores_non_object_json_and_reads_every_data_line() {
    let events = iter_sse_events(&SseChunk::Bytes(
        b"data: [1,2]\n\ndata: {\"index\":2}\ndata: not json\n\n".to_vec(),
    ));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["index"], 2);
}

#[test]
fn text_reader_joins_blocks_and_keeps_the_last_data_line() {
    let source = "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"a\"}}\n\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"ignored\"}}\n\nevent: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"b\"}}\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"c\"}}\n\n";
    assert_eq!(extract_text_from_sse(source.as_bytes()), "ac");
}

#[test]
fn text_reader_ignores_other_event_types_and_non_text_deltas() {
    let source = "event: content_block_start\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"x\"}}\n\nevent: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\nevent: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\"}}\n\n";
    assert_eq!(extract_text_from_sse(source.as_bytes()), "");
}

#[test]
fn text_reader_parses_crlf_lines() {
    let source = "event: content_block_delta\r\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\r\n\r\n";
    assert_eq!(extract_text_from_sse(source.as_bytes()), "hi");
}

#[test]
fn has_ended_needs_the_message_delta_event_type_and_a_non_null_reason() {
    let wrong_type = SseChunk::Bytes(
        b"event: content_block_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n"
            .to_vec(),
    );
    let null_reason = SseChunk::Event(
        serde_json::from_value(json!({"type":"message_delta","delta":{"stop_reason":null}}))
            .unwrap(),
    );
    assert!(!streaming_has_ended(&[wrong_type, null_reason]));
}

#[test]
fn fingerprints_ignore_non_tool_use_content_blocks() {
    let items = [
        SseChunk::Bytes(
            b"data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n"
                .to_vec(),
        ),
        SseChunk::Bytes(
            b"data: {\"type\":\"content_block_delta\",\"content_block\":{\"type\":\"tool_use\"}}\n\n".to_vec(),
        ),
    ];
    assert!(streamed_tool_use_fingerprints(&items).is_empty());
}

#[test]
fn iter_events_trims_line_and_payload_whitespace() {
    let chunk = SseChunk::Bytes(b"\x1c \tdata: \t{\"index\":2}\r\x1f\n".to_vec());
    assert_eq!(
        iter_sse_events(&chunk),
        vec![serde_json::from_value::<Map<String, Value>>(json!({"index":2_i64})).unwrap()]
    );
}

#[rstest::rstest]
#[case::absent(json!({}))]
#[case::other_type(json!({"type":"other", "delta":{"stop_reason":"end_turn"}}))]
#[case::null_delta(json!({"type":"message_delta", "delta":null}))]
#[case::no_reason(json!({"type":"message_delta", "delta":{}}))]
fn has_ended_rejects_non_terminal_events(#[case] event: Value) {
    let chunk = SseChunk::Event(event.as_object().unwrap().clone());
    assert!(!streaming_has_ended(&[chunk]));
}

#[test]
fn has_ended_accepts_empty_reason_and_last_event_line() {
    let chunk = SseChunk::Bytes(
        b"event: other\nevent: message_delta\ndata: {\"delta\":{\"stop_reason\":\"\"}}\n\n"
            .to_vec(),
    );
    assert!(streaming_has_ended(&[chunk]));
}

#[test]
fn text_and_end_skip_bad_json_and_missing_payloads() {
    let chunk = SseChunk::Bytes(b"event: content_block_delta\ndata: bad\n\nevent: content_block_delta\ndata: \n\nevent: message_delta\ndata: bad\n\nevent: message_delta\ndata: \n\nevent: content_block_delta\n\nevent: message_delta\n\nevent: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"kept\"}}\n\n".to_vec());
    assert_eq!(streaming_string_so_far(slice::from_ref(&chunk)), "kept");
    assert!(!streaming_has_ended(&[chunk, SseChunk::Other]));
}

#[test]
fn scan_key_without_tools_never_marks_calls_in_flight() {
    let key = streaming_scan_key(&[]);
    assert_eq!(key.texts, [""]);
    assert!(key.tool_calls.is_empty());
    assert!(!key.stream_ended);
    assert!(!key.tool_calls_in_flight);
}

#[test]
fn block_state_reopens_indices_and_retains_highest_seen() {
    let chunk = SseChunk::Bytes(b"data: {\"type\":\"content_block_start\",\"index\":0}\ndata: {\"type\":\"content_block_stop\",\"index\":0}\ndata: {\"type\":\"content_block_start\",\"index\":-1}\ndata: {\"type\":\"content_block_stop\",\"index\":-1}\ndata: {\"type\":\"content_block_start\",\"index\":-1}\n".to_vec());
    assert_eq!(content_block_state(&[chunk]), (Some(-1), Some(0)));
    assert_eq!(content_block_state(&[]), (None, None));
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(mut log) = self.0.lock() {
            log.extend_from_slice(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

#[test]
fn text_and_end_log_malformed_json_and_utf8() {
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(logs.clone())
        .finish();
    let _guard = set_default(subscriber);
    assert_eq!(
        extract_text_from_sse(b"event: content_block_delta\ndata: bad\n\n"),
        ""
    );
    assert!(!streaming_has_ended(&[SseChunk::Bytes(
        b"event: message_delta\ndata: bad\n\n".to_vec()
    )]));
    assert_eq!(extract_text_from_sse(&[0xff]), "");
    assert!(!streaming_has_ended(&[SseChunk::Bytes(vec![0xff])]));
    let output = {
        let bytes = logs.0.lock().unwrap();
        str::from_utf8(&bytes).unwrap().to_owned()
    };
    assert_eq!(
        output
            .matches("WARN Failed to parse JSON from SSE data")
            .count(),
        2
    );
    assert!(output.contains("ERROR Error extracting text from SSE"));
    assert!(output.contains("ERROR Error checking streaming end in SSE"));
}
