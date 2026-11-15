#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::absolute_paths,
        clippy::shadow_unrelated,
        reason = "decoder fixtures exercise wire bytes and the recorded corpus"
    )
)]

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_to_chat_stream::error::ResponsesChatStreamError;
use crate::api::responses_to_chat_stream::records::PaddingChunk;
use crate::api::responses_to_chat_stream::{DecodedEventLine, decode_event_line};

use super::support::event;

#[rstest::rstest]
#[case::empty("")]
#[case::event_line("event: response.created")]
fn event_line_padding_covers_empty_and_event_lines(#[case] line: &str) {
    let decoded = decode_event_line(line.as_bytes(), &ApiLimits::default()).unwrap();
    assert_eq!(
        decoded,
        DecodedEventLine::Padding(PaddingChunk {
            text: String::new(),
            tool_use: None,
            is_finished: false,
            finish_reason: String::new(),
            usage: None,
        })
    );
}

#[test]
fn event_line_removes_everything_through_the_first_data_prefix() {
    let parsed =
        decode_event_line(br#"prefix data:{"type":"unknown"}"#, &ApiLimits::default()).unwrap();
    assert_eq!(
        parsed,
        DecodedEventLine::Event(event(r#"{"type":"unknown"}"#))
    );
    let parsed = decode_event_line(
        br#"data: {"type": "response.output_text.delta", "delta": "hi"}"#,
        &ApiLimits::default(),
    )
    .unwrap();
    assert_eq!(
        parsed,
        DecodedEventLine::Event(event(
            r#"{"type": "response.output_text.delta", "delta": "hi"}"#
        ))
    );
}

#[test]
fn event_line_rejects_invalid_json_with_the_json_decode_error_text() {
    let failure = decode_event_line(b"{", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"
    );
    let done = decode_event_line(b"[DONE]", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        done.to_string(),
        "Expecting value: line 1 column 2 (char 1)"
    );
    let prefixed = decode_event_line(b"data: [DONE]", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        prefixed.to_string(),
        "Expecting value: line 1 column 3 (char 2)"
    );
    let spaces = decode_event_line(b"   ", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        spaces.to_string(),
        "Expecting value: line 1 column 4 (char 3)"
    );
    let array = decode_event_line(b"[DONE, 1]", &ApiLimits::default()).unwrap_err();
    assert_eq!(
        array.to_string(),
        "Expecting value: line 1 column 2 (char 1)"
    );
}

#[test]
fn event_line_json_error_keeps_its_source() {
    let failure = decode_event_line(b"{", &ApiLimits::default()).unwrap_err();
    assert!(std::error::Error::source(&failure).is_some());
}

#[test]
fn event_line_checks_the_event_byte_limit() {
    let limits = ApiLimits {
        max_json_bytes: 16,
        max_event_bytes: 4,
        max_items: 16,
    };
    let failure = decode_event_line(b"data: {}", &limits).unwrap_err();
    assert!(matches!(failure, ResponsesChatStreamError::Limit(_)));
}
