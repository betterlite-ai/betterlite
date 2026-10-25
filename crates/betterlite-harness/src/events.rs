mod tests;

use std::sync::Arc;

use rust_decimal::Decimal;
use tokio::sync::SetOnce;

use crate::types::{FileChangeKind, JsonObject, StopReason, Usage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub kind: FileChangeKind,
    pub diff: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

// the first answer wins and any thread may give it, like LiteLLM's future resolved through call_soon_threadsafe
#[derive(Clone, Debug)]
pub struct Approval {
    pub tool: String,
    pub input: JsonObject,
    pub decision: Arc<SetOnce<Decision>>,
}

#[derive(Clone, Debug)]
pub struct RunResult {
    pub text: String,
    pub output: Option<JsonObject>,
    pub files: Vec<FileChange>,
    pub events: Vec<Event>,
    pub usage: Usage,
    pub cost: Decimal,
    pub stop_reason: StopReason,
    pub session_id: String,
}

#[derive(Clone, Debug)]
pub enum Event {
    Text {
        delta: String,
    },
    Reasoning {
        delta: String,
    },
    ToolCall {
        id: String,
        name: String,
        native_name: String,
        input: JsonObject,
        builtin: bool,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    FileChange(FileChange),
    Compaction {
        tokens_before: Option<u64>,
        tokens_after: Option<u64>,
    },
    Approval(Approval),
    Done {
        result: RunResult,
    },
}
