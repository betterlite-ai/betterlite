#![cfg(test)]

use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::BytesMut;

use crate::event_frames::AwsEvent;
use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::error::EventFrameError;

pub(super) fn frame(entries: &[(&str, HeaderValue)], payload: &[u8]) -> Vec<u8> {
    let headers = entries
        .iter()
        .map(|(name, value)| Header::new((*name).to_owned(), value.clone()))
        .collect();
    let message = Message::new_from_parts(headers, payload.to_vec());
    let mut out = Vec::new();
    write_message_to(&message, &mut out).unwrap();
    out
}

pub(super) fn text(value: &str) -> HeaderValue {
    HeaderValue::String(StrBytes::from(value.to_owned()))
}

pub(super) fn event_frame(event_type: &str, payload: &[u8]) -> Vec<u8> {
    frame(
        &[
            (":event-type", text(event_type)),
            (":message-type", text("event")),
        ],
        payload,
    )
}

pub(super) fn chunk_frame(body: &str) -> Vec<u8> {
    let payload = format!("{{\"bytes\":\"{}\"}}", STANDARD.encode(body));
    event_frame("chunk", payload.as_bytes())
}

pub(super) fn error_frame(kind: &str, exception_type: Option<&str>, payload: &[u8]) -> Vec<u8> {
    let mut headers = vec![(":message-type", text(kind))];
    if let Some(name) = exception_type {
        headers.push((":exception-type", text(name)));
    }
    frame(&headers, payload)
}

pub(super) fn decode_one(bytes: &[u8]) -> Result<Option<AwsEvent>, EventFrameError> {
    let mut buffer = BytesMut::from(bytes);
    decode_aws_event_frame(&mut buffer)
}

pub(super) fn event(bytes: &[u8]) -> AwsEvent {
    decode_one(bytes).unwrap().unwrap()
}

pub(super) fn flip(bytes: &mut [u8], at: usize) {
    bytes[at] ^= 0xff;
}

pub(super) fn bedrock_parts(failure: EventFrameError) -> (u16, String) {
    let EventFrameError::Bedrock { status, error } = failure else {
        panic!("expected a Bedrock error, got {failure:?}");
    };
    (status, error.error.message)
}

pub(super) fn kind(error: &EventFrameError) -> &'static str {
    match error {
        EventFrameError::PreludeChecksum { .. } => "PreludeChecksum",
        EventFrameError::Frame(_) => "Frame",
        EventFrameError::DuplicateHeader { .. } => "DuplicateHeader",
        EventFrameError::NotUtf8(_) => "NotUtf8",
        EventFrameError::NotJson(_) => "NotJson",
        EventFrameError::PayloadNotObject => "PayloadNotObject",
        EventFrameError::ChunkBytesNotText => "ChunkBytesNotText",
        EventFrameError::ChunkBase64(_) => "ChunkBase64",
        EventFrameError::Bedrock { .. } => "Bedrock",
        EventFrameError::Header(_) => "Header",
    }
}
