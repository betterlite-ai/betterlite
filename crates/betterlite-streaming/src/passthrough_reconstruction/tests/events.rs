use crate::passthrough_reconstruction::error::EventError;
use crate::passthrough_reconstruction::events::{
    RawSseChunk, extract_sse_data, split_sse_chunk_into_events,
};

use super::support::text_chunk;

const TWO_EVENTS: &str = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: ping\ndata: {\"type\":\"ping\"}";

fn encoded(event: &str) -> String {
    serde_json::to_string(&extract_sse_data(event).expect("the event carries JSON")).unwrap()
}

#[test]
fn splits_a_chunk_into_its_events() {
    let events = split_sse_chunk_into_events(&text_chunk(TWO_EVENTS)).unwrap();
    assert_eq!(
        events,
        vec![
            "event: message_start\ndata: {\"type\":\"message_start\"}",
            "event: ping\ndata: {\"type\":\"ping\"}",
        ]
    );
}

#[test]
fn trims_each_event_and_drops_blank_pieces() {
    let events =
        split_sse_chunk_into_events(&text_chunk("\n\n  \n\n\n  event: ping  \n\n\n")).unwrap();
    assert_eq!(events, vec!["event: ping"]);
}

#[test]
fn decodes_byte_chunks_as_utf8() {
    let chunk = RawSseChunk::Bytes("data: {\"text\":\"snow \u{e9}\"}".as_bytes().to_vec());
    assert_eq!(chunk.text().unwrap(), "data: {\"text\":\"snow \u{e9}\"}");
    assert_eq!(
        split_sse_chunk_into_events(&chunk).unwrap(),
        vec!["data: {\"text\":\"snow \u{e9}\"}"]
    );
}

#[test]
fn refuses_bytes_that_are_not_utf8() {
    let chunk = RawSseChunk::Bytes(vec![0x66, 0xff, 0xfe]);
    assert!(matches!(chunk.text(), Err(EventError::Utf8(_))));
    assert!(matches!(
        split_sse_chunk_into_events(&chunk),
        Err(EventError::Utf8(_))
    ));
}

#[test]
fn reads_the_payload_of_the_first_data_line() {
    assert_eq!(
        encoded("event: message_start\ndata: {\"type\":\"message_start\",\"n\":1}"),
        "{\"type\":\"message_start\",\"n\":1}"
    );
    assert_eq!(
        encoded("data: {\"type\":\"first\"}\ndata: {\"type\":\"second\"}"),
        "{\"type\":\"first\"}"
    );
    assert_eq!(
        encoded("x data: 1\ndata: {\"type\":\"real\"}"),
        "{\"type\":\"real\"}"
    );
}

#[test]
fn answers_none_for_empty_done_or_malformed_payloads() {
    assert!(extract_sse_data("data:").is_none());
    assert!(extract_sse_data("data:   ").is_none());
    assert!(extract_sse_data("data: [DONE]").is_none());
    assert!(extract_sse_data("data: not json").is_none());
    assert!(extract_sse_data("event: message_start").is_none());
    assert!(extract_sse_data("metadata: {\"type\":\"sneaky\"}").is_none());
    assert!(extract_sse_data("data:\ndata: {\"type\":\"x\"}").is_none());
    assert!(extract_sse_data("data: [DONE]\ndata: {\"type\":\"x\"}").is_none());
    assert!(extract_sse_data("data: nope\ndata: {\"type\":\"x\"}").is_none());
}

#[test]
fn reads_no_data_line_split_on_a_bare_carriage_return() {
    assert!(extract_sse_data("event: x\rdata: {\"type\":\"x\"}").is_none());
}
