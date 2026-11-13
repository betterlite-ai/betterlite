use std::collections::BTreeMap;

use serde::Serialize;

use super::extract::extract_response_text;
use super::wire::{Part, TaskResponse};
use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;

mod tests;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseMessage {
    pub kind: &'static str,
    pub role: &'static str,
    pub parts: Vec<Part>,
    pub message_id: String,
}

pub fn transform_to_a2a_response<S>(
    response: &TaskResponse,
    request_id: &str,
    source: &mut S,
) -> Event<ResponseMessage>
where
    S: EventSource,
{
    let extracted = extract_response_text(response, source);
    let parts = if extracted.parts.is_empty() {
        vec![Part {
            kind: Some("text".to_owned()),
            text: Some(extracted.text),
            extra: BTreeMap::new(),
        }]
    } else {
        extracted.parts
    };
    Event::from((
        request_id.to_owned(),
        ResponseMessage {
            kind: "message",
            role: "agent",
            parts,
            message_id: extracted.message_id,
        },
    ))
}
