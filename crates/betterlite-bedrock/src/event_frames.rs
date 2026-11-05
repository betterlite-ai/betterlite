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

const PRELUDE_BYTES: usize = 12;
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
    let Some((prelude, _)) = buffer.split_first_chunk::<PRELUDE_BYTES>() else {
        return Ok(None);
    };
    let total_length = check_prelude(*prelude)?;
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
fn check_prelude(prelude: [u8; PRELUDE_BYTES]) -> Result<u32, EventFrameError> {
    let [t0, t1, t2, t3, h0, h1, h2, h3, c0, c1, c2, c3] = prelude;
    let total_length = u32::from_be_bytes([t0, t1, t2, t3]);
    let headers_length = u32::from_be_bytes([h0, h1, h2, h3]);
    let actual = u32::from_be_bytes([c0, c1, c2, c3]);
    let expected = crc32fast::hash(&[t0, t1, t2, t3, h0, h1, h2, h3]);
    if expected != actual {
        return Err(EventFrameError::PreludeChecksum { expected, actual });
    }
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
    Ok(total_length)
}

mod tests;
