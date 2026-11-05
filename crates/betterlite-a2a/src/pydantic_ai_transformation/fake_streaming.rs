use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::time::Duration;

use events::{StreamEvent, StreamIds, TaskState};

use super::extract::extract_response_text;
use super::wire::{Part, TaskResponse};
use crate::completion_bridge::EventSource;

pub mod events;
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamPacing {
    pub chunk_size: NonZeroUsize,
    pub delay: Duration,
}

impl Default for StreamPacing {
    fn default() -> Self {
        Self {
            // NonZeroUsize::new(50) cannot be built in a const without unwrapping
            chunk_size: NonZeroUsize::MIN.saturating_add(49),
            delay: Duration::from_millis(10),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulatedEvent {
    pub event: StreamEvent,
    pub delay_after: Duration,
}

pub fn fake_streaming_from_response<S>(
    response: &TaskResponse,
    request_id: &str,
    pacing: StreamPacing,
    source: &mut S,
) -> Vec<SimulatedEvent>
where
    S: EventSource,
{
    let extracted = extract_response_text(response, source);
    let input_message = response
        .result
        .history
        .iter()
        .find(|message| message.role == "user")
        .cloned()
        .unwrap_or_default();
    let ids = StreamIds {
        request_id: request_id.to_owned(),
        task_id: source.next_uuid().to_string(),
        context_id: source.next_uuid().to_string(),
        artifact_id: source.next_uuid().to_string(),
    };
    let input_message_id = input_message
        .message_id
        .unwrap_or_else(|| source.next_uuid().to_string());
    let input_parts = input_message.parts.unwrap_or_else(|| {
        vec![Part {
            kind: Some("text".to_owned()),
            text: Some(String::new()),
            extra: BTreeMap::new(),
        }]
    });
    let mut events = vec![
        SimulatedEvent {
            event: ids.task_event(input_message_id, input_parts),
            delay_after: Duration::ZERO,
        },
        SimulatedEvent {
            event: ids.status_event(TaskState::Working, false),
            delay_after: pacing.delay,
        },
    ];
    events.extend(artifact_events(&ids, &extracted.text, pacing));
    events.push(SimulatedEvent {
        event: ids.status_event(TaskState::Completed, true),
        delay_after: Duration::ZERO,
    });
    events
}

fn artifact_events(ids: &StreamIds, text: &str, pacing: StreamPacing) -> Vec<SimulatedEvent> {
    let characters = text.chars().collect::<Vec<_>>();
    let mut chunks = characters.chunks(pacing.chunk_size.get()).peekable();
    let mut events = Vec::new();
    while let Some(chunk) = chunks.next() {
        let more_chunks = chunks.peek().is_some();
        events.push(SimulatedEvent {
            event: ids.artifact_event(chunk.iter().collect()),
            delay_after: if more_chunks {
                pacing.delay
            } else {
                Duration::ZERO
            },
        });
    }
    events
}
