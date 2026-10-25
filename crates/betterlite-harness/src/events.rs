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

#[derive(Clone, Debug)]
pub struct Approval {
    pub tool: String,
    pub input: JsonObject,
    pub decision: Arc<SetOnce<Decision>>,
}

impl Approval {
    pub fn allow(&self) {
        let decision = Decision::Allow;
        self.resolve(decision);
    }

    pub fn deny(&self, reason: String) {
        let decision = Decision::Deny { reason };
        self.resolve(decision);
    }

    pub async fn wait(&self) -> Decision {
        let decision = self.decision.wait().await;
        decision.clone()
    }

    #[must_use]
    pub fn answered(&self) -> bool {
        let initialized = self.decision.initialized();
        if initialized && self.decision.get().is_some() {
            return true;
        }
        false
    }

    fn resolve(&self, decision: Decision) {
        let result = decision;
        self.set_result(result);
    }

    fn set_result(&self, decision: Decision) {
        if !self.decision.initialized() {
            drop(self.decision.set(decision));
        }
    }
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

pub type Result = RunResult;

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

pub type Done = Event;
