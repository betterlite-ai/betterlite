use std::fs;
use std::str::from_utf8;
use std::time::Duration;

use rstest::rstest;

use crate::oci_chat::response::decode_chat_body;
use crate::oci_chat::stream::{
    MAX_LINE_BYTES, STREAMING_TIMEOUT, SseDecoder, StreamChunk, StreamItem, check_stream_status,
};

const TEXT_LINE: &str = r#"data: {"message":{"content":[{"type":"TEXT","text":"he"}]}}"#;
const COHERE_TOOL_CALL: &str = r#""toolCalls":[{"name":"lookup","parameters":{}}]"#;

fn chunk_at(items: &[StreamItem], position: usize) -> &StreamChunk {
    let StreamItem::Chunk(found) = &items[position] else {
        panic!("the line carries a chunk");
    };
    found
}

fn cohere_line(fields: &str) -> String {
    format!("data: {{\"apiFormat\":\"COHERE\",\"index\":0,{fields}}}\n")
}

#[test]
fn sse_decoder_waits_for_a_full_line() {
    let mut decoder = SseDecoder::default();
    assert!(decoder.push(TEXT_LINE).unwrap().is_empty());
    let rest = decoder.push("\ndata: [DONE]\n").unwrap();
    assert_eq!(rest.len(), 2);
    assert_eq!(
        chunk_at(&rest, 0).choice.delta.content.as_deref(),
        Some("he")
    );
    assert!(matches!(rest[1], StreamItem::Done));
}

#[test]
fn sse_decoder_flushes_a_trailing_line() {
    let mut decoder = SseDecoder::default();
    assert!(decoder.push(TEXT_LINE).unwrap().is_empty());
    let flushed = decoder.finish().unwrap();
    assert_eq!(flushed.len(), 1);
    assert_eq!(
        chunk_at(&flushed, 0).choice.delta.content.as_deref(),
        Some("he")
    );
    assert!(decoder.finish().unwrap().is_empty());
}

#[test]
fn sse_decoder_reports_tool_calls_and_finish_reason() {
    let mut decoder = SseDecoder::default();
    let calling = decoder
        .push(r#"data: {"index":0,"message":{"toolCalls":[{"name":"lookup","arguments":"{}"}]},"finishReason":"TOOL_CALL"}"#)
        .unwrap();
    assert!(calling.is_empty());
    let called = decoder.push("\n").unwrap();
    assert!(chunk_at(&called, 0).tool_call);
    assert_eq!(
        chunk_at(&called, 0).finish_reason.as_deref(),
        Some("tool_calls")
    );
    let plain = decoder
        .push("data: {\"index\":0,\"message\":{\"content\":[]}}\n")
        .unwrap();
    assert!(!chunk_at(&plain, 0).tool_call);
    assert!(chunk_at(&plain, 0).finish_reason.is_none());
}

#[test]
fn sse_decoder_suppresses_cohere_restatements() {
    let mut decoder = SseDecoder::default();
    let first = decoder.push(&cohere_line(r#""text":"hi""#)).unwrap();
    assert_eq!(
        chunk_at(&first, 0).choice.delta.content.as_deref(),
        Some("hi")
    );
    let restated = decoder
        .push(&cohere_line(r#""text":"hi","chatHistory":[]"#))
        .unwrap();
    assert!(chunk_at(&restated, 0).choice.delta.content.is_none());
}

#[test]
fn sse_decoder_marks_cohere_text_only_after_real_content() {
    let mut decoder = SseDecoder::default();
    decoder.push(&cohere_line(r#""text":"""#)).unwrap();
    let later = decoder
        .push(&cohere_line(r#""text":"x","chatHistory":[]"#))
        .unwrap();
    assert_eq!(
        chunk_at(&later, 0).choice.delta.content.as_deref(),
        Some("x")
    );
}

#[test]
fn sse_decoder_marks_cohere_tool_calls_once() {
    let mut decoder = SseDecoder::default();
    decoder.push(&cohere_line(r#""text":"a""#)).unwrap();
    let call = cohere_line(&format!(r#""chatHistory":[],{COHERE_TOOL_CALL}"#));
    let first = decoder.push(&call).unwrap();
    assert!(chunk_at(&first, 0).tool_call);
    let again = decoder.push(&call).unwrap();
    assert!(!chunk_at(&again, 0).tool_call);
}

#[test]
fn sse_decoder_handles_other_lines_and_bad_json() {
    let mut decoder = SseDecoder::default();
    assert!(
        decoder
            .push("event: ping\n: comment\n\n")
            .unwrap()
            .is_empty()
    );
    let refusal = decoder.push("data: {oops}\n").unwrap_err();
    assert_eq!(refusal.status, 500);
    assert!(
        refusal
            .message
            .starts_with("Chunk cannot be parsed as JSON")
    );
}

#[test]
fn sse_decoder_accepts_a_line_of_exactly_the_cap_across_reads() {
    let mut decoder = SseDecoder::default();
    let half: usize = 0x0008_0000;
    assert!(decoder.push(&"x".repeat(half)).unwrap().is_empty());
    assert!(
        decoder
            .push(&"x".repeat(MAX_LINE_BYTES - half))
            .unwrap()
            .is_empty()
    );
    assert!(decoder.push("\n").unwrap().is_empty());
}

#[test]
fn sse_decoder_rejects_an_unfinished_line_past_the_cap() {
    let mut decoder = SseDecoder::default();
    let half: usize = 0x0008_0000;
    assert!(decoder.push(&"x".repeat(half)).unwrap().is_empty());
    let refusal = decoder
        .push(&"x".repeat(MAX_LINE_BYTES - half + 1))
        .unwrap_err();
    assert_eq!(refusal.status, 500);
    assert_eq!(
        refusal.message,
        "the OCI stream line is longer than 1048576 bytes"
    );
}

#[test]
fn sse_decoder_rejects_a_finished_line_past_the_cap() {
    let mut decoder = SseDecoder::default();
    let refusal = decoder
        .push(&format!("{}\n", "x".repeat(MAX_LINE_BYTES + 1)))
        .unwrap_err();
    assert_eq!(refusal.status, 500);
    assert_eq!(
        refusal.message,
        "the OCI stream line is longer than 1048576 bytes"
    );
}

#[rstest]
#[case::ok(200, "", None)]
#[case::unavailable(503, "unavailable", Some("unavailable"))]
#[case::created(201, "made", Some("made"))]
fn stream_status_maps_non_200(
    #[case] status: u16,
    #[case] text: &str,
    #[case] message: Option<&str>,
) {
    let outcome = check_stream_status(status, text);
    assert_eq!(outcome.is_err(), message.is_some());
    if let Some(expected) = message {
        let refusal = outcome.unwrap_err();
        assert_eq!(refusal.status, status);
        assert_eq!(refusal.message, expected);
    }
}

#[test]
fn streaming_timeout_is_300_seconds() {
    assert_eq!(STREAMING_TIMEOUT, Duration::from_secs(300));
}

#[test]
fn replays_oci_chat_bytes_corpus() {
    let seen = fs::read_dir("fuzz/corpus/oci_chat_bytes")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _accepted = decode_chat_body(200, &bytes);
            let _rejected = decode_chat_body(500, &bytes);
            if let Ok(text) = from_utf8(&bytes) {
                let mut decoder = SseDecoder::default();
                let _pushed = decoder.push(text);
                let _finished = decoder.finish();
            }
        })
        .count();
    assert!(seen > 0_usize);
}
