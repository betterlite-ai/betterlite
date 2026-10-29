use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::Number;

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

// a value LiteLLM reads through Python truthiness and str(), never a nested object or array
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum ScalarJson {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
}

impl ScalarJson {
    pub(super) fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(flag) => *flag,
            Self::Number(number) => number.as_f64().is_some_and(|value| value != 0.0_f64),
            Self::Text(text) => !text.is_empty(),
        }
    }

    pub(super) fn python_str(&self) -> String {
        match self {
            Self::Null => "None".to_owned(),
            Self::Bool(flag) => python_bool_str(*flag),
            Self::Number(number) => number.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

fn python_bool_str(flag: bool) -> String {
    if flag { "True" } else { "False" }.to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum FailureDetail {
    Object {
        #[serde(default)]
        message: Option<ScalarJson>,
    },
    Scalar(ScalarJson),
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

pub(super) use wire::{Item, parse, tool_projection};

pub(super) fn item_id(item: &Item) -> String {
    let id = match item {
        Item::AgentMessage { id, .. }
        | Item::Reasoning { id, .. }
        | Item::CommandExecution { id, .. }
        | Item::FileChange { id, .. }
        | Item::WebSearch { id, .. }
        | Item::McpToolCall { id, .. } => id.as_ref(),
        Item::Other => None,
    };
    id.filter(|value| value.is_truthy())
        .map_or_else(String::new, ScalarJson::python_str)
}

#[path = "wire.rs"]
mod wire;
