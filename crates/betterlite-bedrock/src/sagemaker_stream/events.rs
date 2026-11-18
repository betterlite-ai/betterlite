use std::collections::VecDeque;
use std::str::from_utf8;

use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use bytes::BytesMut;

use crate::event_frames::AwsEvent;
use crate::event_frames::decode_aws_event_frame;
use crate::sagemaker_stream::error::SagemakerStreamError;
use crate::sagemaker_stream::project::{self, Projection};
use crate::sagemaker_stream::{ErrorPolicy, SagemakerChunk};

pub type EventItem = Result<Option<SagemakerChunk>, SagemakerStreamError>;

// AWSEventStreamDecoder: one event adds its payload text to the JSON buffer, and a complete object is projected
pub struct EventState {
    pub model: String,
    pub is_messages_api: bool,
    pub policy: ErrorPolicy,
    pub frames: BytesMut,
    pub accumulated: String,
}

impl EventState {
    // false ends the stream: a frame or a projection failed under Propagate
    pub fn absorb(&mut self, bytes: &[u8], pending: &mut VecDeque<EventItem>) -> bool {
        self.frames.extend_from_slice(bytes);
        loop {
            let event = match decode_aws_event_frame(&mut self.frames) {
                Ok(Some(event)) => event,
                Ok(None) => return true,
                Err(source) => {
                    pending.push_back(Err(SagemakerStreamError::Frame(source)));
                    return false;
                }
            };
            if !self.event(&event, pending) {
                return false;
            }
        }
    }

    pub fn finish(&mut self, pending: &mut VecDeque<EventItem>) {
        let Some(item) = self.leftover() else {
            return;
        };
        pending.push_back(item);
    }

    // the remaining buffer is parsed once: a chunk, a None for an unparseable text, or the policy's error
    fn leftover(&mut self) -> Option<EventItem> {
        if self.accumulated.is_empty() {
            return None;
        }
        match project::project(&self.model, self.is_messages_api, &self.accumulated) {
            Ok(Projection::Ready(chunk)) => {
                self.accumulated.clear();
                Some(Ok(Some(*chunk)))
            }
            Ok(Projection::Pending) => {
                tracing::error!(
                    "Warning: Unparseable JSON data remained: {}",
                    self.accumulated
                );
                Some(Ok(None))
            }
            Err(error) if self.policy == ErrorPolicy::Propagate => Some(Err(error)),
            Err(error) => {
                tracing::error!("Final error parsing accumulated JSON: {error}");
                None
            }
        }
    }

    fn event(&mut self, event: &AwsEvent, pending: &mut VecDeque<EventItem>) -> bool {
        let text = match event_text(event) {
            Ok(Some(text)) => text,
            Ok(None) => return true,
            Err(error) => return self.failed(error, pending),
        };
        let message = strip_sse_data_from_chunk(Some(text))
            .unwrap_or_default()
            .replace("\n\n", "");
        self.accumulated.push_str(&message);
        // the buffer is cleared only when the text parses and projects, as the pinned iterator clears it after the yield
        match project::project(&self.model, self.is_messages_api, &self.accumulated) {
            Ok(Projection::Ready(chunk)) => {
                self.push_chunk(*chunk, pending);
                true
            }
            Ok(Projection::Pending) => true,
            Err(error) => self.failed(error, pending),
        }
    }

    fn push_chunk(&mut self, chunk: SagemakerChunk, pending: &mut VecDeque<EventItem>) {
        self.accumulated.clear();
        pending.push_back(Ok(Some(chunk)));
    }

    fn failed(&self, error: SagemakerStreamError, pending: &mut VecDeque<EventItem>) -> bool {
        match self.policy {
            ErrorPolicy::Propagate => {
                pending.push_back(Err(error));
                false
            }
            ErrorPolicy::Skip => {
                if matches!(error, SagemakerStreamError::Utf8(_)) {
                    tracing::warn!(
                        "UnicodeDecodeError: {error}. Attempting to combine with next event."
                    );
                } else {
                    tracing::error!(
                        "Error parsing message: {error}. Attempting to combine with next event."
                    );
                }
                true
            }
        }
    }
}

// _parse_message_from_event: an exception frame is a status failure, an empty payload yields nothing
fn event_text(event: &AwsEvent) -> Result<Option<&str>, SagemakerStreamError> {
    if matches!(
        event.header_text(":message-type"),
        Some("error" | "exception")
    ) {
        return Err(SagemakerStreamError::Status {
            payload: payload_text(&event.payload),
        });
    }
    if event.payload.is_empty() {
        return Ok(None);
    }
    from_utf8(&event.payload)
        .map(Some)
        .map_err(SagemakerStreamError::Utf8)
}

fn payload_text(payload: &[u8]) -> String {
    from_utf8(payload).map_or_else(
        |_not_text| format!("{} bytes that are not text", payload.len()),
        str::to_owned,
    )
}
mod tests;
