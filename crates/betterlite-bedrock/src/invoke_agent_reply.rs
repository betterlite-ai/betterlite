use bytes::BytesMut;

use crate::event_frames::AwsEvent;
use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::error::EventFrameError;
use crate::event_frames::shape::ResponseStreamShape;
use crate::invoke_agent_reply::events::{AgentEvent, EventHeaders, decode_frame, extract_headers};

pub mod error;
pub mod events;
pub mod extract;
pub mod reply;
pub mod wire;

mod tests;

// Mirrors AmazonInvokeAgentConfig._parse_aws_event_stream: every frame botocore's EventStreamBuffer yields, decoded one at a time.
pub fn decode_agent_trace(raw_content: &[u8]) -> Result<Vec<AgentEvent>, EventFrameError> {
    let shape = ResponseStreamShape::bedrock_runtime();
    let mut buffer = BytesMut::from(raw_content);
    let mut events = Vec::new();
    while let Some(frame) = decode_aws_event_frame(&mut buffer)? {
        let headers = headers_or_empty(&frame);
        if let Some(event) = decode_frame(&shape, &frame, &headers) {
            events.push(event);
        }
    }
    Ok(events)
}

// _extract_headers_from_event swallows its own failure and keeps the empty headers, so the frame is still classified.
fn headers_or_empty(frame: &AwsEvent) -> EventHeaders {
    match extract_headers(frame) {
        Ok(headers) => headers,
        Err(error) => {
            tracing::debug!("Error extracting headers: {error}");
            EventHeaders::default()
        }
    }
}
