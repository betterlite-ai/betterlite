use std::collections::BTreeMap;

use aws_smithy_eventstream::frame::read_message_from;
use aws_smithy_types::event_stream::HeaderValue;
use bytes::{Bytes, BytesMut};

use error::EventFrameError;

pub mod beta;
pub mod error;
pub mod json;
pub mod message;
pub mod shape;
pub mod sse;
pub mod tally;

// B173's bounded-buffer policy; botocore allows 24 MiB payloads and 128 KiB of headers
pub const MAX_FRAME_BYTES: u32 = 0x0100_0000;
pub const MAX_HEADER_BYTES: u32 = 0x0001_0000;
const UPSTREAM_BAD_GATEWAY: u16 = 502;

#[derive(Clone, Debug, PartialEq)]
pub struct AwsEvent {
    pub headers: BTreeMap<String, HeaderValue>,
    pub payload: Bytes,
    pub total_length: u32,
}

impl AwsEvent {
    #[must_use]
    pub fn header_text(&self, name: &str) -> Option<&str> {
        let value = self.headers.get(name)?;
        let text = value.as_string().ok()?;
        Some(text.as_str())
    }
}

// Takes the next whole frame off the front of `buffer`; `None` means more bytes are needed.
pub fn decode_aws_event_frame(buffer: &mut BytesMut) -> Result<Option<AwsEvent>, EventFrameError> {
    let Some(total_length) = check_prelude(buffer.as_ref())? else {
        return Ok(None);
    };
    let frame_length = usize::try_from(total_length).unwrap_or(usize::MAX);
    if buffer.len() < frame_length {
        return Ok(None);
    }
    let frame = buffer.split_to(frame_length).freeze();
    let message = read_message_from(frame).map_err(EventFrameError::Frame)?;
    let mut headers = BTreeMap::new();
    for header in message.headers() {
        let name = header.name().as_str().to_owned();
        if headers
            .insert(name.clone(), header.value().clone())
            .is_some()
        {
            return Err(EventFrameError::DuplicateHeader { name });
        }
    }
    Ok(Some(AwsEvent {
        headers,
        payload: message.payload().clone(),
        total_length,
    }))
}

// Checks the prelude CRC and the size limits before any frame bytes are buffered; returns the total length.
fn check_prelude(buffer: &[u8]) -> Result<Option<u32>, EventFrameError> {
    let Some((total_length_bytes, after_total)) = buffer.split_first_chunk::<4>() else {
        return Ok(None);
    };
    let Some((headers_length_bytes, after_headers)) = after_total.split_first_chunk::<4>() else {
        return Ok(None);
    };
    let Some((checksum_bytes, _)) = after_headers.split_first_chunk::<4>() else {
        return Ok(None);
    };
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(total_length_bytes);
    hasher.update(headers_length_bytes);
    let expected = hasher.finalize();
    let actual = u32::from_be_bytes(*checksum_bytes);
    if expected != actual {
        return Err(EventFrameError::PreludeChecksum { expected, actual });
    }
    let total_length = u32::from_be_bytes(*total_length_bytes);
    let headers_length = u32::from_be_bytes(*headers_length_bytes);
    if total_length > MAX_FRAME_BYTES {
        return Err(EventFrameError::upstream(
            UPSTREAM_BAD_GATEWAY,
            format!(
                "Bedrock event-stream frame of {total_length} bytes exceeds the {MAX_FRAME_BYTES} byte limit"
            ),
        ));
    }
    if headers_length > MAX_HEADER_BYTES {
        return Err(EventFrameError::upstream(
            UPSTREAM_BAD_GATEWAY,
            format!(
                "Bedrock event-stream headers of {headers_length} bytes exceed the {MAX_HEADER_BYTES} byte limit"
            ),
        ));
    }
    Ok(Some(total_length))
}

mod tests;
