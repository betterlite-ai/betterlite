#![cfg(test)]

use std::collections::BTreeMap;

use aws_smithy_types::DateTime;
use aws_smithy_types::event_stream::HeaderValue;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use bytes::{Bytes, BytesMut};

use super::frames::{bedrock_parts, chunk_frame, decode_one, event, flip, frame, kind, text};
use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::error::EventFrameError;
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;
use crate::event_frames::tally::EventStreamTally;

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

fn request_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "content-type".to_owned(),
            "application/vnd.amazon.eventstream".to_owned(),
        ),
        ("x-amzn-requestid".to_owned(), "req-1".to_owned()),
    ])
}

pub(super) fn split_frame() {
    let shape = ResponseStreamShape::bedrock_runtime();
    let mut wire = chunk_frame("hello");
    wire.extend(chunk_frame("world"));
    for cut in 0..=wire.len() {
        let mut buffer = BytesMut::new();
        let mut texts = Vec::new();
        for part in [&wire[..cut], &wire[cut..]] {
            buffer.extend_from_slice(part);
            while let Some(found) = decode_aws_event_frame(&mut buffer).unwrap() {
                texts.push(parse_message_from_event(Some(&shape), &found).unwrap());
            }
        }
        assert_eq!(texts, [Some("hello".to_owned()), Some("world".to_owned())]);
        assert!(buffer.is_empty(), "cut {cut}");
    }
}

pub(super) fn crc_mismatch() {
    let good = chunk_frame("hello");
    let mut prelude_bad = good.clone();
    flip(&mut prelude_bad, 11);
    let (prelude, _) = prelude_bad.split_at(12);
    assert_eq!(kind(&decode_one(prelude).unwrap_err()), "PreludeChecksum");
    let mut message_bad = good.clone();
    flip(&mut message_bad, good.len().saturating_sub(1));
    assert_eq!(kind(&decode_one(&message_bad).unwrap_err()), "Frame");
    let mut header_bad = good;
    flip(&mut header_bad, 14);
    assert_eq!(kind(&decode_one(&header_bad).unwrap_err()), "Frame");
}

pub(super) fn header_type_values() {
    let uuid = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef_u128;
    let headers = [
        ("a-true", HeaderValue::Bool(true)),
        ("b-false", HeaderValue::Bool(false)),
        ("c-byte", HeaderValue::Byte(-5)),
        ("d-int16", HeaderValue::Int16(-300)),
        ("e-int32", HeaderValue::Int32(70_000)),
        ("f-int64", HeaderValue::Int64(-5_000_000_000)),
        (
            "g-bytes",
            HeaderValue::ByteArray(Bytes::from_static(b"\x00\xff")),
        ),
        ("h-string", text("value")),
        (
            "i-time",
            HeaderValue::Timestamp(DateTime::from_millis(1_700_000_000_123)),
        ),
        ("j-uuid", HeaderValue::Uuid(uuid)),
    ];
    let decoded = event(&frame(&headers, b"{}"));
    for (name, value) in headers {
        assert_eq!(decoded.headers.get(name), Some(&value), "{name}");
    }
    assert_eq!(decoded.header_text("h-string"), Some("value"));
    assert_eq!(decoded.header_text("c-byte"), None);
}

pub(super) fn truncated_frame() {
    let whole = chunk_frame("hello");
    let (partial, _) = whole.split_at(whole.len().saturating_sub(1));
    let mut buffer = BytesMut::from(partial);
    assert!(decode_aws_event_frame(&mut buffer).unwrap().is_none());
    assert_eq!(buffer.len(), partial.len());
    let mut tally = EventStreamTally::default();
    tally.add_chunk(partial);
    let error = tally
        .undecoded_stream_error(None, &redactor())
        .map(bedrock_parts)
        .unwrap();
    assert_eq!(error.0, 502);
    assert!(error.1.contains("decoded to no events"), "{}", error.1);
    assert!(
        error
            .1
            .contains(&format!("{} bytes received", partial.len()))
    );
    let mut short = vec![0, 0, 0, 8, 0, 0, 0, 0];
    short.extend(crc32fast::hash(&short).to_be_bytes());
    assert_eq!(kind(&decode_one(&short).unwrap_err()), "Frame");
}

pub(super) fn error_before_first_event() {
    let redactor = redactor();
    let empty = EventStreamTally::default();
    no_events_message(&empty, &redactor);
    first_bytes_message(&redactor);
    textual_diagnostics(&empty, &redactor);
}

fn no_events_message(empty: &EventStreamTally, redactor: &Redactor) {
    let (status, first) = bedrock_parts(empty.undecoded_stream_error(None, redactor).unwrap());
    assert_eq!(status, 502);
    assert_eq!(
        first,
        "Bedrock answered the stream with HTTP 200 but its body decoded to no events (content-type=None, x-amzn-requestid=None, 0 bytes received, first bytes=b'')"
    );
}

fn first_bytes_message(redactor: &Redactor) {
    let mut tally = EventStreamTally::default();
    tally.add_chunk(&[b'x'; 150]);
    tally.add_chunk(b"y\n\r\t\\'\"\x01\xff");
    tally.add_chunk(&[b'z'; 60]);
    let headers = request_headers();
    let error = tally
        .undecoded_stream_error(Some(&headers), redactor)
        .unwrap();
    let (_, second) = bedrock_parts(error);
    let mut head = String::from("b'");
    head.push_str(&"x".repeat(150));
    head.push('y');
    head.push_str(r"\n");
    head.push_str(r"\r");
    head.push_str(r"\t");
    head.push_str(r"\\");
    head.push_str(r"\'");
    head.push('"');
    head.push_str(r"\x01");
    head.push_str(r"\xff");
    head.push_str(&"z".repeat(41));
    head.push('\'');
    assert!(
        second.ends_with(&format!("first bytes={head})")),
        "{second}"
    );
    assert!(second.contains("content-type='application/vnd.amazon.eventstream', x-amzn-requestid='req-1', 219 bytes received"));
}

fn textual_diagnostics(empty: &EventStreamTally, redactor: &Redactor) {
    let escaped_headers = BTreeMap::from([
        (
            "content-type".to_owned(),
            "a\\b'c\"d\ne\rf\tg\u{1}h".to_owned(),
        ),
        ("x-amzn-requestid".to_owned(), "it's".to_owned()),
    ]);
    let (_, escaped) = bedrock_parts(
        empty
            .undecoded_stream_error(Some(&escaped_headers), redactor)
            .unwrap(),
    );
    let mut expected = String::from("content-type='a");
    expected.push_str(r"\\");
    expected.push('b');
    expected.push_str(r"\'");
    expected.push_str("c\"d");
    expected.push_str(r"\n");
    expected.push('e');
    expected.push_str(r"\r");
    expected.push('f');
    expected.push_str(r"\t");
    expected.push('g');
    expected.push_str(r"\x01");
    expected.push_str("h', x-amzn-requestid=\"it's\", 0 bytes received");
    assert!(escaped.contains(&expected), "{escaped}");
    let upper = BTreeMap::from([("Content-Type".to_owned(), "x".to_owned())]);
    let (_, third) = bedrock_parts(
        empty
            .undecoded_stream_error(Some(&upper), redactor)
            .unwrap(),
    );
    assert!(third.contains("content-type=None"), "{third}");
}

pub(super) fn leftover_frame_after_events() {
    let redactor = redactor();
    let wire = chunk_frame("hello");
    let found = event(&wire);
    let mut whole = EventStreamTally::default();
    whole.add_chunk(&wire);
    whole.add_event(&found);
    assert!(whole.undecoded_stream_error(None, &redactor).is_none());
    let mut leftover = EventStreamTally::default();
    leftover.add_chunk(&wire);
    leftover.add_chunk(b"trailing");
    leftover.add_event(&found);
    let headers = request_headers();
    let error = leftover
        .undecoded_stream_error(Some(&headers), &redactor)
        .unwrap();
    let (status, message) = bedrock_parts(error);
    assert_eq!(status, 502);
    let received = wire.len().saturating_add(8);
    assert_eq!(
        message,
        format!(
            "Bedrock stream ended with 8 undecoded bytes after 1 events (content-type='application/vnd.amazon.eventstream', x-amzn-requestid='req-1', {received} bytes received)"
        )
    );
    assert!(!message.contains("first bytes"));
    let mut unfinished = EventStreamTally::default();
    unfinished.add_chunk(&wire);
    assert!(matches!(
        unfinished.undecoded_stream_error(None, &redactor),
        Some(EventFrameError::Bedrock { status: 502, .. })
    ));
}
