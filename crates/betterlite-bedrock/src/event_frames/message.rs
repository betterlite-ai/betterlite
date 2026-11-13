use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::AwsEvent;
use super::error::EventFrameError;
use super::json::{BodyField, parse_body_object};
use super::shape::{ResponseStreamShape, StreamMember};

const SHAPE_MISSING_STATUS: u16 = 500;
const RAW_ERROR_STATUS: u16 = 400;

// Mirrors BedrockEventStreamDecoderBase._parse_message_from_event: the text of one event, or None for an empty one.
pub fn parse_message_from_event(
    shape: Option<&ResponseStreamShape>,
    event: &AwsEvent,
) -> Result<Option<String>, EventFrameError> {
    let Some(known) = shape else {
        return Err(EventFrameError::upstream(
            SHAPE_MISSING_STATUS,
            "Bedrock event-stream shape could not be loaded from botocore. Ensure botocore is correctly installed."
                .to_owned(),
        ));
    };
    if matches!(
        event.header_text(":message-type"),
        Some("error" | "exception")
    ) {
        return Err(stream_error(known, event));
    }
    let member = event
        .header_text(":event-type")
        .and_then(|name| known.members.get(name));
    if member == Some(&StreamMember::Chunk) {
        return chunk_text(event);
    }
    if event.payload.is_empty() {
        return Ok(None);
    }
    let text = from_utf8(&event.payload).map_err(EventFrameError::NotUtf8)?;
    Ok(Some(text.to_owned()))
}

fn chunk_text(event: &AwsEvent) -> Result<Option<String>, EventFrameError> {
    let object = parse_body_object(&event.payload)?;
    let encoded = match object.get("bytes") {
        None | Some(BodyField::Null(())) => return Ok(None),
        Some(BodyField::Text(text)) => text,
        Some(BodyField::Other(_)) => return Err(EventFrameError::ChunkBytesNotText),
    };
    let decoded = STANDARD
        .decode(encoded)
        .map_err(EventFrameError::ChunkBase64)?;
    let text = from_utf8(&decoded).map_err(EventFrameError::NotUtf8)?;
    Ok(Some(text.to_owned()))
}

// Mirrors build_bedrock_stream_error: the modeled member's httpStatusCode replaces botocore's fixed 400.
fn stream_error(shape: &ResponseStreamShape, event: &AwsEvent) -> EventFrameError {
    let exception_type = event.header_text(":exception-type");
    let member = exception_type.and_then(|name| shape.members.get(name));
    if member.is_some()
        && let Err(source) = parse_body_object(&event.payload)
    {
        return source;
    }
    let body = match from_utf8(&event.payload) {
        Ok(text) => text,
        Err(source) => return EventFrameError::NotUtf8(source),
    };
    let message = match exception_type {
        Some(name) if !name.is_empty() => format!("{name} {body}"),
        _ => body.to_owned(),
    };
    let status = member
        .and_then(|found| found.http_status())
        .unwrap_or(RAW_ERROR_STATUS);
    EventFrameError::upstream(status, message)
}
