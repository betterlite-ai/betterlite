use std::collections::VecDeque;

use betterlite_prompts::arguments::json::Json;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use bytes::BytesMut;

use crate::event_frames::AwsEvent;
use crate::event_frames::decode_aws_event_frame;
use crate::sagemaker_stream::error::SagemakerStreamError;
use crate::sagemaker_stream::project;
use crate::sagemaker_stream::{ErrorPolicy, SagemakerChunk};

pub type EventItem = Result<Option<SagemakerChunk>, SagemakerStreamError>;

// AWSEventStreamDecoder: one event adds its payload text to the JSON buffer, and a complete object is projected
pub struct EventState {
    model: String,
    is_messages_api: bool,
    policy: ErrorPolicy,
    frames: BytesMut,
    accumulated: String,
}

impl EventState {
    #[must_use]
    pub fn new(model: &str, is_messages_api: bool, policy: ErrorPolicy) -> Self {
        Self {
            model: model.to_owned(),
            is_messages_api,
            policy,
            frames: BytesMut::new(),
            accumulated: String::new(),
        }
    }

    // false ends the stream: a frame or a projection failed under Propagate
    pub fn absorb(&mut self, bytes: &[u8], pending: &mut VecDeque<EventItem>) -> bool {
        self.frames.extend_from_slice(bytes);
        loop {
            match decode_aws_event_frame(&mut self.frames) {
                Ok(Some(event)) => {
                    if !self.event(&event, pending) {
                        return false;
                    }
                }
                Ok(None) => return true,
                Err(source) => {
                    pending.push_back(Err(SagemakerStreamError::Frame(source)));
                    return false;
                }
            }
        }
    }

    pub fn finish(&mut self, pending: &mut VecDeque<EventItem>) {
        if self.accumulated.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_str(&self.accumulated) else {
            tracing::error!(
                "Warning: Unparseable JSON data remained: {}",
                self.accumulated
            );
            pending.push_back(Ok(None));
            return;
        };
        self.project_at_end(&value, pending);
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
        self.project_accumulated(pending)
    }

    // the buffer is cleared only when the JSON parses and projects, as the pinned iterator clears it after the yield
    fn project_accumulated(&mut self, pending: &mut VecDeque<EventItem>) -> bool {
        let Ok(value) = serde_json::from_str(&self.accumulated) else {
            return true;
        };
        self.push_projection(&value, pending)
    }

    fn push_projection(&mut self, value: &Json, pending: &mut VecDeque<EventItem>) -> bool {
        let chunk = match self.project(value) {
            Ok(chunk) => chunk,
            Err(error) => return self.failed(error, pending),
        };
        self.push_chunk(chunk, pending);
        true
    }

    // a buffer that still parses here failed its projection on arrival, so the same failure returns
    fn project_at_end(&mut self, value: &Json, pending: &mut VecDeque<EventItem>) {
        match self.project(value) {
            Ok(chunk) => self.push_chunk(chunk, pending),
            Err(error) if self.policy == ErrorPolicy::Propagate => pending.push_back(Err(error)),
            Err(error) => tracing::error!("Final error parsing accumulated JSON: {error}"),
        }
    }

    fn push_chunk(&mut self, chunk: SagemakerChunk, pending: &mut VecDeque<EventItem>) {
        self.accumulated.clear();
        pending.push_back(Ok(Some(chunk)));
    }

    fn project(&self, value: &Json) -> Result<SagemakerChunk, SagemakerStreamError> {
        if self.is_messages_api {
            return project::chat_chunk(&self.model, value).map(SagemakerChunk::Chat);
        }
        project::token_chunk(value).map(SagemakerChunk::Token)
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
    std::str::from_utf8(&event.payload)
        .map(Some)
        .map_err(SagemakerStreamError::Utf8)
}

fn payload_text(payload: &[u8]) -> String {
    std::str::from_utf8(payload).map_or_else(
        |_not_text| format!("{} bytes that are not text", payload.len()),
        str::to_owned,
    )
}
mod tests;
