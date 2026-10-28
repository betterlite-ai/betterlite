use std::collections::BTreeSet;

use serde::Deserialize;

#[derive(Clone, Debug, Default)]
pub struct CodexStreamState {
    pub thread_id: Option<String>,
    pub final_text: String,
    pub error: Option<String>,
    pub failed: bool,
    pub started: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ItemEventKind {
    Started,
    Updated,
    Completed,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum FailureDetail {
    Object {
        #[serde(default)]
        message: Option<String>,
    },
    Text(String),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum StreamEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted {
        #[serde(default)]
        thread_id: Option<String>,
    },
    #[serde(rename = "item.started")]
    ItemStarted { item: Item },
    #[serde(rename = "item.updated")]
    ItemUpdated { item: Item },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: Item },
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        message: Option<String>,
    },
    #[serde(rename = "turn.failed")]
    TurnFailed {
        #[serde(default)]
        error: Option<FailureDetail>,
    },
    #[serde(other)]
    Other,
}

pub(super) use wire::{Item, parse, tool_input, tool_output};

pub(super) fn item_id(item: &Item) -> &str {
    match item {
        Item::AgentMessage { id, .. }
        | Item::Reasoning { id, .. }
        | Item::CommandExecution { id, .. }
        | Item::FileChange { id, .. }
        | Item::WebSearch { id, .. }
        | Item::McpToolCall { id, .. } => id,
        Item::Other => "",
    }
}

#[path = "wire.rs"]
mod wire;
