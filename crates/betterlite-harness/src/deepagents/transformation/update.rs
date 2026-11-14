use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::errors::HarnessError;
use crate::events::Event;
use crate::types::JsonObject;

use super::EVENT_NODES;
use super::message::{Message, MessageKind, content_text, tool_call_event};

pub const INTERRUPT_KEY: &str = "__interrupt__";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Update {
    pub entries: BTreeMap<String, UpdateEntry>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum UpdateEntry {
    #[default]
    Other,
    Messages(Vec<Message>),
    // LangGraph reports interrupts under INTERRUPT_KEY instead of a node name
    Interrupts(Vec<Interrupt>),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Interrupt {
    pub id: String,
    pub value: InterruptValue,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InterruptValue {
    pub action_requests: Vec<ActionRequest>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActionRequest {
    pub name: String,
    pub args: JsonObject,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HitlDecision {
    Approve,
    Reject { message: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TurnState {
    pub interrupts: Vec<Interrupt>,
}

#[must_use]
pub fn decision(allowed: bool, reason: &str) -> HitlDecision {
    if allowed {
        return HitlDecision::Approve;
    }
    if reason.is_empty() {
        return HitlDecision::Reject {
            message: "The user denied this tool call.".to_owned(),
        };
    }
    HitlDecision::Reject {
        message: reason.to_owned(),
    }
}

#[must_use]
pub fn interrupts_in(update: &Update) -> Vec<Interrupt> {
    update
        .entries
        .values()
        .filter_map(|entry| match entry {
            UpdateEntry::Interrupts(interrupts) => Some(interrupts.as_slice()),
            UpdateEntry::Messages(_) | UpdateEntry::Other => None,
        })
        .flatten()
        .cloned()
        .collect()
}

fn message_events(message: &Message, skip_tools: &BTreeSet<String>) -> Vec<Event> {
    match message.kind {
        MessageKind::Ai => message
            .tool_calls
            .iter()
            .filter(|call| !skip_tools.contains(&call.name))
            .map(tool_call_event)
            .collect(),
        MessageKind::Tool => {
            if message
                .name
                .as_ref()
                .is_some_and(|name| skip_tools.contains(name))
            {
                return Vec::new();
            }
            vec![Event::ToolResult {
                id: message.tool_call_id.clone(),
                output: content_text(&message.content),
                is_error: message.is_error,
            }]
        }
        MessageKind::AiMessageChunk
        | MessageKind::Human
        | MessageKind::System
        | MessageKind::Other => Vec::new(),
    }
}

fn node_messages<'entry>(node: &str, entry: &'entry UpdateEntry) -> &'entry [Message] {
    match entry {
        UpdateEntry::Messages(messages) if EVENT_NODES.contains(&node) => messages,
        UpdateEntry::Messages(_) | UpdateEntry::Interrupts(_) | UpdateEntry::Other => &[],
    }
}

#[must_use]
pub fn update_events(update: &Update, skip_tools: &BTreeSet<String>) -> Vec<Event> {
    update
        .entries
        .iter()
        .flat_map(|(node, entry)| node_messages(node, entry))
        .flat_map(|message| message_events(message, skip_tools))
        .collect()
}

pub fn structured_json(value: Option<&JsonObject>) -> Result<Option<String>, HarnessError> {
    if let Some(object) = value {
        let text =
            serde_json::to_string(object).map_err(|source| HarnessError::StateEncode { source })?;
        return Ok(Some(text));
    }
    Ok(None)
}
