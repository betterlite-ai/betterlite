use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use wire::{
    A2AMessage, CompletionParams, ModelResponse, OpenAIMessage, ResponseMessage,
    SendMessageResponse, TextPart,
};

pub mod events;
pub mod provider;
pub mod streaming;
mod tests;
pub mod wire;

pub trait EventSource {
    fn next_uuid(&mut self) -> Uuid;
    fn now(&mut self) -> DateTime<Utc>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct A2AStreamingContext {
    pub request_id: String,
    pub task_id: String,
    pub context_id: String,
    pub input_message: A2AMessage,
    pub accumulated_text: String,
    pub has_emitted_task: bool,
    pub has_emitted_working: bool,
}

impl A2AStreamingContext {
    pub fn new<S>(request_id: String, input_message: A2AMessage, source: &mut S) -> Self
    where
        S: EventSource,
    {
        let task_id = source.next_uuid().to_string();
        let context_id = source.next_uuid().to_string();
        Self {
            request_id,
            task_id,
            context_id,
            input_message,
            accumulated_text: String::new(),
            has_emitted_task: false,
            has_emitted_working: false,
        }
    }
}

#[must_use]
pub fn extract_text(parts: &[AgentJson]) -> String {
    let texts = parts.iter().filter_map(|item| {
        let part = item.as_object()?;
        if let Some(kind) = part.get("kind")
            && !kind.is_null()
            && kind.as_str() != Some("")
            && kind.as_str() != Some("text")
        {
            return None;
        }
        part.get("text").and_then(AgentJson::as_str)
    });
    texts.collect::<Vec<_>>().join("\n")
}

#[must_use]
pub fn forward_metadata(
    message: &A2AMessage,
    params: Option<&CompletionParams>,
) -> Option<BTreeMap<String, AgentJson>> {
    let mut merged = BTreeMap::new();
    if let Some(metadata) = params
        .and_then(|value| value.metadata.as_ref())
        .and_then(AgentJson::as_object)
    {
        merged.extend(metadata.clone());
    }
    if let Some(metadata) = message.metadata.as_ref().and_then(AgentJson::as_object) {
        merged.extend(metadata.clone());
    }
    (!merged.is_empty()).then_some(merged)
}

pub fn apply_forward_metadata(
    completion: &mut CompletionParams,
    message: &A2AMessage,
    params: Option<&CompletionParams>,
) {
    let Some(forwarded) = forward_metadata(message, params) else {
        return;
    };
    let mut extra_body = completion
        .extra_body
        .as_ref()
        .and_then(AgentJson::as_object)
        .cloned()
        .unwrap_or_default();
    let mut metadata = forwarded;
    if let Some(existing) = extra_body.get("metadata").and_then(AgentJson::as_object) {
        metadata.extend(existing.clone());
    }
    extra_body.insert(
        "metadata".to_owned(),
        AgentJson::Object(metadata.into_iter().collect()),
    );
    completion.extra_body = Some(AgentJson::Object(extra_body));
}

#[must_use]
pub fn to_openai_messages(message: &A2AMessage) -> Vec<OpenAIMessage> {
    let content = extract_text(&message.parts);
    vec![OpenAIMessage {
        role: message.role.0.clone(),
        content,
    }]
}

pub fn response_to_a2a<S>(
    response: Option<&ModelResponse>,
    request_id: Option<&str>,
    source: &mut S,
) -> SendMessageResponse
where
    S: EventSource,
{
    let content = response
        .and_then(|model| model.choices.first())
        .and_then(|choice| choice.message.as_ref())
        .and_then(|message| message.content.as_deref())
        .unwrap_or_default()
        .to_owned();
    SendMessageResponse {
        jsonrpc: "2.0".to_owned(),
        id: request_id.map(str::to_owned),
        result: ResponseMessage {
            kind: "message".to_owned(),
            role: "agent".to_owned(),
            parts: vec![TextPart {
                kind: "text".to_owned(),
                text: content,
            }],
            message_id: source.next_uuid().simple().to_string(),
        },
    }
}
