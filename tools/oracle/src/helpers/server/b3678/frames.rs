use aws_smithy_eventstream::frame::{read_message_from, write_message_to};
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use serde_json::Value;

use super::FrameInput;
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

pub(super) fn encode_stream(
    frames: &[FrameInput],
    trailing_hex: &str,
) -> Result<Vec<u8>, Box<HelperError>> {
    let mut body = Vec::new();
    for frame in frames {
        let payload = serde_json::to_vec(&frame.payload).or_value_error()?;
        let header = Header::new(
            ":event-type".to_owned(),
            HeaderValue::String(StrBytes::from(frame.event_type.clone())),
        );
        let message = Message::new(payload).add_header(header);
        write_message_to(&message, &mut body).or_value_error()?;
    }
    body.extend_from_slice(&hex::decode(trailing_hex).or_value_error()?);
    Ok(body)
}

pub(super) fn decode_stream(body: &[u8]) -> HelperResult {
    let mut frames = Vec::new();
    let mut offset = 0_usize;
    while let Some(total) = frame_total_length(body, offset) {
        let end = offset.saturating_add(total);
        let Some(raw) = body.get(offset..end) else {
            break;
        };
        let message = read_message_from(raw).or_value_error()?;
        frames.push(frame_json(&message)?);
        offset = end;
    }
    let trailing = body.get(offset..).unwrap_or_default();
    Ok(serde_json::json!({
        "frames": frames,
        "trailing": hex::encode(trailing),
    }))
}

fn frame_total_length(body: &[u8], offset: usize) -> Option<usize> {
    let (length_bytes, _rest) = body.get(offset..)?.split_first_chunk::<4>()?;
    let total = usize::try_from(u32::from_be_bytes(*length_bytes)).ok()?;
    if total < 16 {
        return None;
    }
    let end = offset.checked_add(total)?;
    (end <= body.len()).then_some(total)
}

fn frame_json(message: &Message) -> Result<Value, Box<HelperError>> {
    let mut headers = serde_json::Map::new();
    for header in message.headers() {
        headers.insert(
            header.name().as_str().to_owned(),
            header_value_json(header.value()),
        );
    }
    let payload = serde_json::from_slice::<Value>(message.payload().as_ref()).or_value_error()?;
    Ok(serde_json::json!({ "headers": headers, "payload": payload }))
}

fn header_value_json(value: &HeaderValue) -> Value {
    value.as_string().map_or_else(
        |_| Value::String(format!("{value:?}")),
        |text| Value::String(text.as_str().to_owned()),
    )
}
