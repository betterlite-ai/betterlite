use std::collections::BTreeMap;

use aws_smithy_eventstream::frame::read_message_from;
use aws_smithy_types::str_bytes::StrBytes;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use crc32fast::hash as crc32;

use super::groups::{GroupKey, delta_text_cells, write_delta_cells};

const PRELUDE_BYTES: usize = 12;
const MESSAGE_CRC_BYTES: usize = 4;
const MIN_FRAME_BYTES: usize = 16;
const EVENT_TYPE_HEADER: &str = ":event-type";
const CONTENT_BLOCK_DELTA: &str = "contentBlockDelta";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub raw: Vec<u8>,
    pub texts: Vec<(GroupKey, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStream {
    pub frames: Vec<Frame>,
    pub trailing: Vec<u8>,
}

#[must_use]
pub fn parse_event_stream(body: &[u8]) -> ParsedStream {
    let mut frames = Vec::new();
    let mut offset = 0_usize;
    while let Some(total_length) = frame_total_length(body, offset) {
        let Some(end) = offset.checked_add(total_length) else {
            break;
        };
        let Some(raw) = body.get(offset..end) else {
            break;
        };
        frames.push(read_frame(raw.to_vec()));
        offset = end;
    }
    let trailing = body.get(offset..).map_or_else(Vec::new, <[u8]>::to_vec);
    ParsedStream { frames, trailing }
}

#[must_use]
pub fn rebuild_event_stream(
    parsed: &ParsedStream,
    new_texts: &BTreeMap<(usize, usize), String>,
) -> Vec<u8> {
    let mut rebuilt = Vec::new();
    for (frame_index, frame) in parsed.frames.iter().enumerate() {
        if frame.texts.is_empty() {
            rebuilt.extend_from_slice(&frame.raw);
            continue;
        }
        let replacement = rebuild_frame(frame_index, frame, new_texts);
        rebuilt.extend_from_slice(replacement.as_deref().unwrap_or(&frame.raw));
    }
    rebuilt.extend_from_slice(&parsed.trailing);
    rebuilt
}

fn frame_total_length(body: &[u8], offset: usize) -> Option<usize> {
    let prelude_end = offset.checked_add(MIN_FRAME_BYTES)?;
    if prelude_end > body.len() {
        return None;
    }
    let (length_bytes, _rest) = body.get(offset..)?.split_first_chunk::<4>()?;
    let total_length = usize::try_from(u32::from_be_bytes(*length_bytes)).ok()?;
    if total_length < MIN_FRAME_BYTES {
        return None;
    }
    let end = offset.checked_add(total_length)?;
    (end <= body.len()).then_some(total_length)
}

fn read_frame(raw: Vec<u8>) -> Frame {
    let message = match read_message_from(raw.as_slice()) {
        Ok(message) => message,
        Err(error) => {
            tracing::debug!(
                "could not decode event-stream frame, forwarding it unmodified: {error}"
            );
            return Frame {
                raw,
                texts: Vec::new(),
            };
        }
    };
    let event_type = message
        .headers()
        .iter()
        .find(|header| header.name().as_str() == EVENT_TYPE_HEADER)
        .and_then(|header| header.value().as_string().ok())
        .map(StrBytes::as_str);
    if event_type != Some(CONTENT_BLOCK_DELTA) {
        return Frame {
            raw,
            texts: Vec::new(),
        };
    }
    let parsed = serde_json::from_slice::<GuardrailValue>(message.payload().as_ref());
    let texts = if let Ok(GuardrailValue::Object(fields)) = parsed {
        fields.get("delta").map_or_else(Vec::new, delta_text_cells)
    } else {
        tracing::debug!("could not parse contentBlockDelta payload, forwarding frame unmodified");
        Vec::new()
    };
    Frame { raw, texts }
}

fn rebuild_frame(
    frame_index: usize,
    frame: &Frame,
    new_texts: &BTreeMap<(usize, usize), String>,
) -> Option<Vec<u8>> {
    let (total_bytes, rest) = frame.raw.split_first_chunk::<4>()?;
    let (headers_bytes, _rest) = rest.split_first_chunk::<4>()?;
    let total_length = usize::try_from(u32::from_be_bytes(*total_bytes)).ok()?;
    let headers_length = usize::try_from(u32::from_be_bytes(*headers_bytes)).ok()?;
    let headers_end = PRELUDE_BYTES.checked_add(headers_length)?;
    let payload_end = total_length.checked_sub(MESSAGE_CRC_BYTES)?;
    let headers = frame.raw.get(PRELUDE_BYTES..headers_end)?;
    let payload_bytes = frame.raw.get(headers_end..payload_end)?;
    let mut payload = serde_json::from_slice::<GuardrailValue>(payload_bytes).ok()?;
    let GuardrailValue::Object(fields) = &mut payload else {
        return None;
    };
    if let Some(delta) = fields.get_mut("delta") {
        write_delta_cells(delta, frame_index, new_texts);
    }
    let new_payload = serde_json::to_vec(&payload).ok()?;
    let new_total = PRELUDE_BYTES
        .checked_add(headers_length)?
        .checked_add(new_payload.len())?
        .checked_add(MESSAGE_CRC_BYTES)?;
    let new_total_u32 = u32::try_from(new_total).ok()?;
    let headers_length_u32 = u32::try_from(headers_length).ok()?;
    let mut rebuilt = Vec::new();
    rebuilt.extend_from_slice(&new_total_u32.to_be_bytes());
    rebuilt.extend_from_slice(&headers_length_u32.to_be_bytes());
    let prelude_crc = crc32(&rebuilt);
    rebuilt.extend_from_slice(&prelude_crc.to_be_bytes());
    rebuilt.extend_from_slice(headers);
    rebuilt.extend_from_slice(&new_payload);
    let message_crc = crc32(&rebuilt);
    rebuilt.extend_from_slice(&message_crc.to_be_bytes());
    Some(rebuilt)
}
