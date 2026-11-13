#![cfg(test)]

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::Path;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::chunk::{ChunkFlag, DecodedMessagesChunk, native_error_status};
use super::render::{ResponseEventsError, incomplete_error_sse_event, response_as_sse_events};
use super::stream::{
    MessagesProviderStream, MessagesStreamHiddenParams, MessagesStreamingResponse,
};
use super::{decode_messages_sse_chunk, decode_messages_sse_value};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};

const INCOMPLETE: &str = "Provider stream ended before emitting a message_stop event; the response is incomplete and any partial content (e.g. tool_use input JSON) may be truncated.";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

fn flag_set(flags: &[ChunkFlag]) -> BTreeSet<ChunkFlag> {
    flags.iter().copied().collect()
}

fn classified(input: &[u8]) -> DecodedMessagesChunk {
    decode_messages_sse_chunk(input, &ApiLimits::default()).expect("the read is within the cap")
}

fn frames(response: &str) -> Vec<String> {
    response_as_sse_events(&json(response))
        .expect("the response renders")
        .into_iter()
        .map(|frame| String::from_utf8(frame).expect("frames are UTF-8"))
        .collect()
}

fn frame(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn expected_frames(pairs: &[(&str, &str)]) -> Vec<String> {
    pairs
        .iter()
        .map(|(event, data)| frame(event, data))
        .collect()
}

#[rstest]
#[case::whole_ping(b"event: ping\n\n".as_slice(), &[ChunkFlag::Ping])]
#[case::ping_with_data(b"event: ping\ndata: {\"type\": \"ping\"}\n\n".as_slice(), &[ChunkFlag::Ping])]
#[case::multiple_pings(b"event: ping\ndata: {}\n\nevent: ping\ndata: {}\n\n".as_slice(), &[ChunkFlag::Ping])]
#[case::crlf_ping(b"event: ping\r\n\r\n".as_slice(), &[ChunkFlag::Ping])]
#[case::ping_with_comment(b"event: ping\n: keepalive\n\n".as_slice(), &[ChunkFlag::Ping])]
#[case::ping_then_content(b"event: ping\ndata: {}\n\nevent: content_block_delta\ndata: {}\n\n".as_slice(), &[ChunkFlag::ContentDelta])]
#[case::leading_frame_tail(b"data: {}\n\nevent: ping\n\n".as_slice(), &[])]
#[case::leading_whitespace(b" event: ping\n\n".as_slice(), &[])]
#[case::missing_final_boundary(b"event: ping".as_slice(), &[])]
#[case::split_ping(b"event: pi".as_slice(), &[])]
#[case::single_newline_boundary(b"event: ping\n".as_slice(), &[])]
#[case::trailing_vertical_tab(b"event: ping\x0b".as_slice(), &[])]
#[case::ping_joined_by_vertical_tab(b"event: ping\x0bevent: ping\n\n".as_slice(), &[])]
#[case::message_stop_line(b"event: message_stop\n".as_slice(), &[ChunkFlag::MessageStop, ChunkFlag::Terminal])]
#[case::message_stop_without_frame(b"event: message_stop".as_slice(), &[ChunkFlag::MessageStop, ChunkFlag::Terminal])]
#[case::message_stop_padded(b"event: message_stop \n".as_slice(), &[])]
#[case::content_delta_line(b"event: content_block_delta".as_slice(), &[ChunkFlag::ContentDelta])]
#[case::stop_and_delta(b"event: content_block_delta\nevent: message_stop\n".as_slice(), &[ChunkFlag::ContentDelta, ChunkFlag::MessageStop, ChunkFlag::Terminal])]
#[case::data_only_stop(b"data: {\"type\": \"message_stop\"}\n\n".as_slice(), &[])]
#[case::data_only_delta(b"data: {\"type\": \"content_block_delta\"}\n\n".as_slice(), &[])]
#[case::data_only_ping(b"data: {\"type\": \"ping\"}\n\n".as_slice(), &[])]
#[case::plain_bytes(b"hello".as_slice(), &[])]
#[case::empty(b"".as_slice(), &[])]
fn raw_chunk_classifiers(#[case] input: &[u8], #[case] expected: &[ChunkFlag]) {
    let decoded = classified(input);
    assert_eq!(decoded.flags, flag_set(expected), "input {input:?}");
}

#[rstest]
#[case::dict_ping(r#"{"type": "ping"}"#, &[ChunkFlag::Ping])]
#[case::dict_message_stop(r#"{"type": "message_stop"}"#, &[ChunkFlag::MessageStop, ChunkFlag::Terminal])]
#[case::dict_content_delta(r#"{"type": "content_block_delta"}"#, &[ChunkFlag::ContentDelta])]
#[case::dict_other(r#"{"type": "message_delta"}"#, &[])]
#[case::string_value(r#""event: ping\n\n""#, &[])]
#[case::number_value("7", &[])]
#[case::null_value("null", &[])]
#[case::list_value(r#"["event: message_stop"]"#, &[])]
fn raw_chunk_classifiers_values(#[case] input: &str, #[case] expected: &[ChunkFlag]) {
    let decoded = decode_messages_sse_value(&json(input));
    assert_eq!(decoded.flags, flag_set(expected), "input {input}");
}

#[rstest]
#[case::event_label_disagrees(b"event: message_delta\ndata: {\"type\": \"content_block_delta\", \"index\": 0}\n".as_slice(), "content_block_delta", Some("{\"type\":\"content_block_delta\",\"index\":0}"))]
#[case::first_match_wins(b"data: {\"type\": \"content_block_delta\", \"index\": 0}\ndata: {\"type\": \"content_block_delta\", \"index\": 1}\n".as_slice(), "content_block_delta", Some("{\"type\":\"content_block_delta\",\"index\":0}"))]
#[case::malformed_then_valid(b"data: not json\ndata: {\"type\": \"stop\", \"ok\": true}\n".as_slice(), "stop", Some("{\"type\":\"stop\",\"ok\":true}"))]
#[case::arrays_scalars_ignored(b"data: [1, 2]\ndata: 5\ndata: \"text\"\ndata: null\ndata: true\ndata: {\"type\": \"stop\"}\n".as_slice(), "stop", Some("{\"type\":\"stop\"}"))]
#[case::wrong_type_then_match(b"data: {\"type\": \"other\"}\ndata: {\"type\": \"stop\", \"n\": 2}\n".as_slice(), "stop", Some("{\"type\":\"stop\",\"n\":2}"))]
#[case::missing_prefix(b"datax: {\"type\": \"stop\"}\nData: {\"type\": \"stop\"}\n data: {\"type\": \"stop\"}\ndata: {\"type\": \"stop\"}\n".as_slice(), "stop", Some("{\"type\":\"stop\"}"))]
#[case::no_payload(b"event: message_stop\n\ndata:\ndata:   \n".as_slice(), "message_stop", None)]
#[case::multiline_not_joined(b"data: {\"type\": \"message_stop\",\ndata: \"x\": 1}\n".as_slice(), "message_stop", None)]
#[case::invalid_utf8(b"data: {\"type\": \"stop\", \"x\": \"\xff\"}\ndata: {\"type\": \"stop\", \"ok\": true}\n".as_slice(), "stop", Some("{\"type\":\"stop\",\"ok\":true}"))]
fn data_line_extraction(#[case] input: &[u8], #[case] event: &str, #[case] expected: Option<&str>) {
    let decoded = classified(input);
    let found = decoded
        .payload(event)
        .map(|payload| serde_json::to_string(payload).expect("a payload serializes"));
    assert_eq!(found.as_deref(), expected, "event {event}");
}

#[rstest]
#[case::surrounding_bytes(b"data:   \t{\"type\": \"stop\"} \x0b\x0c\r\n".as_slice(), Some("{\"type\":\"stop\"}"))]
#[case::utf8_bom(b"data: \xef\xbb\xbf{\"type\": \"stop\"}\n".as_slice(), Some("{\"type\":\"stop\"}"))]
fn data_line_stripping(#[case] input: &[u8], #[case] expected: Option<&str>) {
    let decoded = classified(input);
    let found = decoded
        .payload("stop")
        .map(|payload| serde_json::to_string(payload).expect("a payload serializes"));
    assert_eq!(found.as_deref(), expected);
}

#[rstest]
#[case::utf16_le("utf-16-le")]
#[case::utf16_be("utf-16-be")]
#[case::utf32_le("utf-32-le")]
#[case::utf32_be("utf-32-be")]
fn data_line_decodes_byte_order_marks(#[case] encoding: &str) {
    let document = r#"{"type": "stop"}"#;
    let encoded = match encoding {
        "utf-16-le" => with_bom(
            &[0xFF, 0xFE],
            &document
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>(),
        ),
        "utf-16-be" => with_bom(
            &[0xFE, 0xFF],
            &document
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<u8>>(),
        ),
        "utf-32-le" => with_bom(
            &[0xFF, 0xFE, 0x00, 0x00],
            &document
                .chars()
                .flat_map(|character| u32::from(character).to_le_bytes())
                .collect::<Vec<u8>>(),
        ),
        "utf-32-be" => with_bom(
            &[0x00, 0x00, 0xFE, 0xFF],
            &document
                .chars()
                .flat_map(|character| u32::from(character).to_be_bytes())
                .collect::<Vec<u8>>(),
        ),
        other => panic!("unexpected encoding {other}"),
    };
    let mut input = b"data: ".to_vec();
    input.extend_from_slice(&encoded);
    input.extend_from_slice(b"\n\n");
    let decoded = classified(&input);
    assert_eq!(
        decoded
            .payload("stop")
            .map(|payload| serde_json::to_string(payload).expect("json")),
        Some("{\"type\":\"stop\"}".to_owned()),
        "{encoding}"
    );
}

fn with_bom(bom: &[u8], encoded: &[u8]) -> Vec<u8> {
    let mut bytes = bom.to_vec();
    bytes.extend_from_slice(encoded);
    bytes
}

#[rstest]
#[case::missing_message(r#"{"type": "error", "error": {"type": "overloaded_error"}}"#, Some(("overloaded_error", "overloaded_error", 503)))]
#[case::string_message(r#"{"type": "error", "error": {"type": "api_error", "message": "boom"}}"#, Some(("api_error", "boom", 500)))]
#[case::empty_message(r#"{"type": "error", "error": {"type": "api_error", "message": ""}}"#, Some(("api_error", "", 500)))]
#[case::nonstring_message(r#"{"type": "error", "error": {"type": "rate_limit_error", "message": 5}}"#, Some(("rate_limit_error", "rate_limit_error", 429)))]
#[case::nonstring_type(r#"{"type": "error", "error": {"type": 5}}"#, None)]
#[case::empty_error_body(r#"{"type": "error", "error": {}}"#, None)]
#[case::nonobject_error(r#"{"type": "error", "error": "boom"}"#, None)]
#[case::unknown_type(r#"{"type": "error", "error": {"type": "vendor_error"}}"#, Some(("vendor_error", "vendor_error", 500)))]
fn native_error_projection(#[case] input: &str, #[case] case_expected: Option<(&str, &str, u16)>) {
    let decoded = decode_messages_sse_value(&json(input));
    let projected = decoded
        .messages_error()
        .map(|error| (error.error_type, error.message, error.status));
    let expected =
        case_expected.map(|(kind, message, status)| (kind.to_owned(), message.to_owned(), status));
    assert_eq!(projected, expected);
}

#[rstest]
#[case::empty_error_body(r#"{"type": "error", "error": {}}"#, true)]
#[case::nonstring_type(r#"{"type": "error", "error": {"type": 5}}"#, true)]
#[case::nonobject_error(r#"{"type": "error", "error": "boom"}"#, false)]
#[case::message_stop(r#"{"type": "message_stop"}"#, true)]
#[case::other(r#"{"type": "message_delta"}"#, false)]
fn terminal_detection(#[case] input: &str, #[case] expected: bool) {
    let decoded = decode_messages_sse_value(&json(input));
    assert_eq!(decoded.flags.contains(&ChunkFlag::Terminal), expected);
}

#[rstest]
#[case::invalid_request_error(400)]
#[case::authentication_error(401)]
#[case::permission_error(403)]
#[case::not_found_error(404)]
#[case::rate_limit_error(429)]
#[case::api_error(500)]
#[case::overloaded_error(503)]
#[case::timeout_error(504)]
fn status_table(#[case] status: u16) {
    let error_type = match status {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        500 => "api_error",
        503 => "overloaded_error",
        504 => "timeout_error",
        other => panic!("unexpected status {other}"),
    };
    assert_eq!(native_error_status(error_type), status);
}

#[test]
fn unknown_error_types_map_to_500() {
    assert_eq!(native_error_status("request_too_large"), 500);
    assert_eq!(native_error_status("vendor_error"), 500);
    assert_eq!(native_error_status(""), 500);
}

#[rstest]
#[case::details_object(r#"{"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": {"category": "cyber"}}}"#, Some("{\"category\":\"cyber\"}"))]
#[case::empty_details(
    r#"{"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": {}}}"#,
    Some("{}")
)]
#[case::no_details(
    r#"{"type": "message_delta", "delta": {"stop_reason": "refusal"}}"#,
    None
)]
#[case::nonobject_details(
    r#"{"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": "x"}}"#,
    None
)]
#[case::other_reason(
    r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_details": {}}}"#,
    None
)]
#[case::delta_not_object(r#"{"type": "message_delta", "delta": "refusal"}"#, None)]
#[case::missing_delta(r#"{"type": "message_delta"}"#, None)]
#[case::other_payload(r#"{"type": "message_stop"}"#, None)]
fn refusal_stop_details(#[case] input: &str, #[case] expected: Option<&str>) {
    let decoded = decode_messages_sse_value(&json(input));
    let found = decoded
        .refusal_stop_details()
        .map(|details| serde_json::to_string(details).expect("a details object serializes"));
    assert_eq!(found.as_deref(), expected);
}

#[test]
fn completed_response_lifecycle() {
    let response = r#"{"type": "message", "id": "msg_a", "model": "claude-sonnet-4-20250514", "role": "assistant", "content": [{"type": "text", "text": "Hi"}], "usage": {"input_tokens": 5, "output_tokens": 2, "cache_read_input_tokens": 3}, "stop_reason": "end_turn", "extra": 7}"#;
    assert_eq!(
        frames(response),
        vec![
            frame(
                "message_start",
                r#"{"type": "message_start", "message": {"type": "message", "id": "msg_a", "model": "claude-sonnet-4-20250514", "role": "assistant", "content": [], "usage": {"input_tokens": 5, "output_tokens": 0, "cache_read_input_tokens": 3}, "stop_reason": null, "extra": 7, "stop_sequence": null}}"#
            ),
            frame(
                "content_block_start",
                r#"{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}"#
            ),
            frame(
                "content_block_delta",
                r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hi"}}"#
            ),
            frame(
                "content_block_stop",
                r#"{"type": "content_block_stop", "index": 0}"#
            ),
            frame(
                "message_delta",
                r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"input_tokens": 5, "output_tokens": 2, "cache_read_input_tokens": 3}}"#
            ),
            frame("message_stop", r#"{"type": "message_stop"}"#),
        ]
    );
}

#[test]
fn completed_response_missing_fields() {
    assert_eq!(
        frames(r#"{"id": "msg_b"}"#),
        vec![
            frame(
                "message_start",
                r#"{"type": "message_start", "message": {"id": "msg_b", "content": [], "stop_reason": null, "stop_sequence": null, "usage": {"output_tokens": 0}}}"#
            ),
            frame(
                "message_delta",
                r#"{"type": "message_delta", "delta": {"stop_reason": null, "stop_sequence": null}, "usage": {}}"#
            ),
            frame("message_stop", r#"{"type": "message_stop"}"#),
        ]
    );
}

#[test]
fn completed_response_null_fields() {
    assert_eq!(
        frames(r#"{"id": "msg_c", "content": null, "usage": null}"#),
        vec![
            frame(
                "message_start",
                r#"{"type": "message_start", "message": {"id": "msg_c", "content": [], "usage": {"output_tokens": 0}, "stop_reason": null, "stop_sequence": null}}"#
            ),
            frame(
                "message_delta",
                r#"{"type": "message_delta", "delta": {"stop_reason": null, "stop_sequence": null}, "usage": {}}"#
            ),
            frame("message_stop", r#"{"type": "message_stop"}"#),
        ]
    );
}

#[test]
fn completed_response_empty_object() {
    assert_eq!(
        frames("{}"),
        vec![
            frame(
                "message_start",
                r#"{"type": "message_start", "message": {"content": [], "stop_reason": null, "stop_sequence": null, "usage": {"output_tokens": 0}}}"#
            ),
            frame(
                "message_delta",
                r#"{"type": "message_delta", "delta": {"stop_reason": null, "stop_sequence": null}, "usage": {}}"#
            ),
            frame("message_stop", r#"{"type": "message_stop"}"#),
        ]
    );
}

const TOOL_USE_FRAMES: [(&str, &str); 12] = [
    (
        "message_start",
        r#"{"type": "message_start", "message": {"content": [], "stop_reason": null, "usage": {"output_tokens": 0}, "stop_sequence": null}}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 0, "content_block": {"id": "tool_1", "name": "lookup", "input": {}, "type": "tool_use"}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 0, "delta": {"partial_json": "{\"x\": 1}", "type": "input_json_delta"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 0}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 1, "content_block": {"id": null, "name": null, "input": {}, "type": "tool_use"}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 1, "delta": {"partial_json": "{}", "type": "input_json_delta"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 1}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 2, "content_block": {"id": null, "name": null, "input": {}, "type": "tool_use"}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 2, "delta": {"partial_json": "{}", "type": "input_json_delta"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 2}"#,
    ),
    (
        "message_delta",
        r#"{"type": "message_delta", "delta": {"stop_reason": "tool_use", "stop_sequence": null}, "usage": {"output_tokens": 4}}"#,
    ),
    ("message_stop", r#"{"type": "message_stop"}"#),
];

const THINKING_FRAMES: [(&str, &str); 12] = [
    (
        "message_start",
        r#"{"type": "message_start", "message": {"content": [], "stop_reason": null, "stop_sequence": null, "usage": {"output_tokens": 0}}}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 0, "content_block": {"thinking": "", "signature": "", "type": "thinking"}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 0, "delta": {"thinking": "Thought", "type": "thinking_delta"}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 0, "delta": {"signature": "sig", "type": "signature_delta"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 0}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 1, "content_block": {"thinking": "", "signature": "", "type": "thinking"}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 1, "delta": {"thinking": "", "type": "thinking_delta"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 1}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 2, "content_block": {"type": "redacted_thinking", "data": "cipher"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 2}"#,
    ),
    (
        "message_delta",
        r#"{"type": "message_delta", "delta": {"stop_reason": null, "stop_sequence": null}, "usage": {}}"#,
    ),
    ("message_stop", r#"{"type": "message_stop"}"#),
];

const OTHER_FRAMES: [(&str, &str); 9] = [
    (
        "message_start",
        r#"{"type": "message_start", "message": {"content": [], "stop_reason": null, "stop_sequence": null, "usage": {"output_tokens": 0}}}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "packed"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 0}"#,
    ),
    (
        "content_block_start",
        r#"{"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}"#,
    ),
    (
        "content_block_delta",
        r#"{"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "\u00e9"}}"#,
    ),
    (
        "content_block_stop",
        r#"{"type": "content_block_stop", "index": 1}"#,
    ),
    (
        "message_delta",
        r#"{"type": "message_delta", "delta": {"stop_reason": null, "stop_sequence": null}, "usage": {}}"#,
    ),
    ("message_stop", r#"{"type": "message_stop"}"#),
];

#[test]
fn completed_response_tool_use_blocks() {
    let response = r#"{"content": [{"type": "tool_use", "id": "tool_1", "name": "lookup", "input": {"x": 1}}, {"type": "tool_use", "input": null}, {"type": "tool_use", "input": false}], "stop_reason": "tool_use", "usage": {"output_tokens": 4}}"#;
    assert_eq!(frames(response), expected_frames(&TOOL_USE_FRAMES));
}

#[test]
fn completed_response_thinking_blocks() {
    let response = r#"{"content": [{"type": "thinking", "thinking": "Thought", "signature": "sig"}, {"type": "thinking", "thinking": "", "signature": ""}, {"type": "redacted_thinking", "data": "cipher"}]}"#;
    assert_eq!(frames(response), expected_frames(&THINKING_FRAMES));
}

#[test]
fn completed_response_other_blocks() {
    let response = "{\"content\": [{\"type\": \"compaction\", \"text\": \"packed\"}, {\"type\": \"text\", \"text\": \"\\u00e9\"}]}";
    assert_eq!(frames(response), expected_frames(&OTHER_FRAMES));
}

#[test]
fn incomplete_error_literal() {
    let frame = String::from_utf8(incomplete_error_sse_event()).expect("the event is UTF-8");
    assert_eq!(
        frame,
        format!(
            "event: error\ndata: {{\"type\": \"error\", \"error\": {{\"type\": \"api_error\", \"message\": \"{INCOMPLETE}\"}}}}\n\n"
        )
    );
    assert!(!frame.contains("param"));
    assert!(!frame.contains("code"));
    assert!(!frame.contains("[DONE]"));
}

#[rstest]
#[case::response_not_object("[]", ResponseEventsError::ResponseNotObject)]
#[case::content_not_list(
    r#"{"content": {"type": "text"}}"#,
    ResponseEventsError::ContentNotList
)]
#[case::block_not_object(r#"{"content": [null]}"#, ResponseEventsError::BlockNotObject { index: 0 })]
#[case::block_not_object_second(r#"{"content": [{"type": "text"}, 5]}"#, ResponseEventsError::BlockNotObject { index: 1 })]
#[case::usage_not_object(r#"{"usage": [1]}"#, ResponseEventsError::UsageNotObject)]
fn response_shape_errors(#[case] input: &str, #[case] expected: ResponseEventsError) {
    assert_eq!(response_as_sse_events(&json(input)), Err(expected));
}

#[rstest]
#[case::null_content("null")]
#[case::false_content("false")]
#[case::zero_content("0")]
#[case::empty_string_content(r#""""#)]
#[case::empty_list_content("[]")]
fn blank_content_emits_no_blocks(#[case] content: &str) {
    let rendered = frames(&format!(r#"{{"content": {content}}}"#));
    assert_eq!(rendered.len(), 3);
    assert!(rendered[0].starts_with("event: message_start\n"));
    assert!(rendered[1].starts_with("event: message_delta\n"));
    assert!(rendered[2].starts_with("event: message_stop\n"));
}

#[rstest]
#[case::null_usage("null")]
#[case::false_usage("false")]
#[case::zero_usage("0")]
#[case::empty_string_usage(r#""""#)]
#[case::empty_list_usage("[]")]
fn blank_usage_takes_the_default(#[case] usage: &str) {
    let rendered = frames(&format!(r#"{{"usage": {usage}}}"#));
    assert!(rendered[0].ends_with("\"usage\": {\"output_tokens\": 0}}}}\n\n"));
    assert!(rendered[1].contains("\"usage\": {}}"));
}

#[test]
fn chunk_limits() {
    let limits = ApiLimits::default();
    let at_cap = vec![b'a'; limits.max_event_bytes];
    decode_messages_sse_chunk(&at_cap, &limits).expect("the read is at the cap");
    let over = vec![b'a'; limits.max_event_bytes.saturating_add(1)];
    let error = decode_messages_sse_chunk(&over, &limits).expect_err("past the cap");
    assert_eq!(
        error,
        ApiLimitError::EventTooLarge {
            observed: limits.max_event_bytes.saturating_add(1),
            limit: limits.max_event_bytes,
        }
    );
    let fields = error.fields();
    assert_eq!(fields.status, 502);
    assert_eq!(fields.error_type, "bad_gateway");
    assert_eq!(fields.param, "stream");
    assert_eq!(fields.code, "event_too_large");
}

#[test]
fn provider_headers_survive_stream() {
    let headers = IndexMap::from([
        ("x-amzn-requestid".to_owned(), "req_a".to_owned()),
        ("x-amzn-trace-id".to_owned(), "trace_a".to_owned()),
        (
            "x-litellm-attempted-fallbacks".to_owned(),
            "spoof".to_owned(),
        ),
        ("content-type".to_owned(), "text/event-stream".to_owned()),
    ]);
    let hidden = MessagesStreamHiddenParams::from(&headers);
    assert_eq!(
        hidden
            .additional_headers
            .get("llm_provider-x-amzn-requestid"),
        Some(&Json::Text("req_a".to_owned()))
    );
    assert_eq!(
        hidden
            .additional_headers
            .get("llm_provider-x-amzn-trace-id"),
        Some(&Json::Text("trace_a".to_owned()))
    );
    assert_eq!(
        hidden
            .additional_headers
            .get("llm_provider-x-litellm-attempted-fallbacks"),
        Some(&Json::Text("spoof".to_owned()))
    );
    assert!(
        !hidden
            .additional_headers
            .contains_key("x-litellm-attempted-fallbacks")
    );
    assert!(
        !hidden
            .additional_headers
            .contains_key("llm_provider-llm_provider-x-amzn-requestid")
    );
}

struct FakeStream {
    chunks: VecDeque<Vec<u8>>,
    close: Option<Result<(), String>>,
    buffered: Option<Json>,
}

impl FakeStream {
    fn with_chunks(chunks: &[&[u8]]) -> Self {
        Self {
            chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
            close: None,
            buffered: None,
        }
    }
}

#[async_trait]
impl MessagesProviderStream for FakeStream {
    type Error = String;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.chunks.pop_front())
    }

    async fn aclose(&mut self) -> Option<Result<(), String>> {
        self.close.take()
    }

    fn buffered_provider_output(&self) -> Option<Json> {
        self.buffered.clone()
    }
}

fn view(stream: FakeStream) -> MessagesStreamingResponse<FakeStream> {
    MessagesStreamingResponse {
        completion_stream: stream,
        hidden_params: MessagesStreamHiddenParams::default(),
    }
}

#[tokio::test]
async fn stream_view_yields_provider_bytes_unchanged() {
    let first = b"event: ping\ndata: {}\n\n";
    let second = b"event: message_stop\ndata: {}\n\n";
    let mut response = view(FakeStream::with_chunks(&[first, second]));
    let mut yielded = Vec::new();
    while let Some(chunk) = response.next_chunk().await.expect("the stream yields") {
        yielded.push(chunk);
    }
    assert_eq!(yielded, vec![first.to_vec(), second.to_vec()]);
    assert_eq!(
        response.hidden_params,
        MessagesStreamHiddenParams::default()
    );
    assert_eq!(response.aclose().await, None);
}

#[tokio::test]
async fn stream_view_awaits_a_supported_close() {
    let mut stream = FakeStream::with_chunks(&[]);
    stream.close = Some(Ok(()));
    let mut response = view(stream);
    assert_eq!(response.aclose().await, Some(Ok(())));
    assert!(response.completion_stream.close.is_none());
}

#[tokio::test]
async fn stream_view_propagates_close_errors() {
    let mut stream = FakeStream::with_chunks(&[]);
    stream.close = Some(Err("close failed".to_owned()));
    let mut response = view(stream);
    assert_eq!(
        response.aclose().await,
        Some(Err("close failed".to_owned()))
    );
}

#[rstest]
#[case::literal_true("true", true)]
#[case::literal_false("false", false)]
#[case::number_one("1", false)]
#[case::number_zero("0", false)]
#[case::truthy_string(r#""yes""#, false)]
#[case::empty_string(r#""""#, false)]
#[case::null("null", false)]
#[case::object("{}", false)]
fn buffered_provider_output_is_the_literal_true(#[case] value: &str, #[case] expected: bool) {
    let mut stream = FakeStream::with_chunks(&[]);
    stream.buffered = Some(json(value));
    let response = view(stream);
    assert_eq!(response.has_buffered_provider_output(), expected);
}

#[test]
fn buffered_provider_output_is_absent_by_default() {
    let response = view(FakeStream::with_chunks(&[]));
    assert!(!response.has_buffered_provider_output());
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/messages_sse_events");
    let replayed = fs::read_dir(corpus)
        .expect("the corpus folder")
        .map(|entry| {
            let bytes = fs::read(entry.expect("an entry").path()).expect("a seed file");
            decode_messages_sse_chunk(&bytes, &ApiLimits::default())
                .expect("the seed is a bounded read");
        })
        .count();
    assert!(replayed >= 1);
}
