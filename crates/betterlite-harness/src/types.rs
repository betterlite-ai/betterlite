use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::errors::StateIncompatible;

pub const STATE_VERSION: u32 = 1;
pub type JsonMap = BTreeMap<String, serde_json::Value>;
pub type StopReason = String;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Harness {
    #[serde(rename = "claude_code")]
    CLAUDE_CODE,
    #[serde(rename = "codex")]
    CODEX,
    #[serde(rename = "opencode")]
    OPENCODE,
    #[serde(rename = "deepagents")]
    DEEPAGENTS,
}

impl Harness {
    pub const fn value(self) -> &'static str {
        match self {
            Self::CLAUDE_CODE => "claude_code",
            Self::CODEX => "codex",
            Self::OPENCODE => "opencode",
            Self::DEEPAGENTS => "deepagents",
        }
    }
}
pub trait HarnessInput {
    fn into_harness(self) -> std::result::Result<Harness, String>;
}

impl HarnessInput for Harness {
    fn into_harness(self) -> std::result::Result<Harness, String> {
        Ok(self)
    }
}
impl HarnessInput for &str {
    fn into_harness(self) -> std::result::Result<Harness, String> {
        let normalized = self.trim().to_ascii_lowercase().replace('-', "_");
        match normalized.as_str() {
            "claude_code" => Ok(Harness::CLAUDE_CODE),
            "codex" => Ok(Harness::CODEX),
            "opencode" => Ok(Harness::OPENCODE),
            "deepagents" => Ok(Harness::DEEPAGENTS),
            _ => Err(format!("harness must be a Harness member, got {self:?}")),
        }
    }
}
pub fn require_harness<T: HarnessInput>(value: T) -> std::result::Result<Harness, String> {
    let result = value.into_harness();
    result
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub calls: u64,
}
impl Usage {
    pub const fn total_tokens(&self) -> u64 {
        let total = self.input_tokens + self.output_tokens;
        total
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub delta: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Reasoning {
    pub delta: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub native_name: String,
    pub input: JsonMap,
    pub builtin: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub output: String,
    pub is_error: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: String,
    pub diff: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Compaction {
    pub tokens_before: Option<u64>,
    pub tokens_after: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Result {
    pub text: String,
    pub output: Option<serde_json::Value>,
    pub files: Vec<FileChange>,
    pub events: Vec<Event>,
    pub usage: Usage,
    pub cost: f64,
    pub stop_reason: StopReason,
    pub session_id: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub result: Result,
}
impl Done {
    pub fn usage(&self) -> &Usage {
        let usage = &self.result.usage;
        usage
    }
    pub const fn cost(&self) -> f64 {
        let cost = self.result.cost;
        cost
    }
    pub fn stop_reason(&self) -> &str {
        let reason = &self.result.stop_reason;
        reason
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(Text),
    Reasoning(Reasoning),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    FileChange(FileChange),
    Compaction(Compaction),
    Approval(Approval),
    Done(Done),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub structured_output: bool,
    pub tool_approval: bool,
    pub tool_filtering: bool,
    pub history: bool,
    pub custom_tools: bool,
    pub skills: bool,
    pub resume: bool,
    pub permission_modes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub harness: Harness,
    pub native_session_id: Option<String>,
    pub workdir: String,
    pub model: Option<String>,
    pub version: u32,
}
impl State {
    pub fn dumps(&self) -> std::result::Result<Vec<u8>, serde_json::Error> {
        let bytes = serde_json::to_vec(self);
        bytes
    }
    pub fn loads(data: &[u8]) -> std::result::Result<Self, StateIncompatible> {
        let state: Self =
            serde_json::from_slice(data).map_err(|error| StateIncompatible(error.to_string()))?;
        if state.version != STATE_VERSION {
            return Err(StateIncompatible(format!(
                "State version {} is not supported",
                state.version
            )));
        }
        Ok(state)
    }
}

#[derive(Clone, Debug)]
pub struct Approval {
    pub tool: String,
    pub input: JsonMap,
    decision: Arc<ApprovalState>,
}
#[derive(Debug)]
struct ApprovalState {
    result: Mutex<Option<(bool, String)>>,
    notify: Notify,
}
impl Approval {
    pub fn new(tool: String, input: JsonMap) -> Self {
        let approval = Self {
            tool,
            input,
            decision: Arc::new(ApprovalState {
                result: Mutex::new(None),
                notify: Notify::new(),
            }),
        };
        approval
    }
    pub fn allow(&self) {
        let reason = String::new();
        self.resolve(true, reason);
    }
    pub fn deny(&self, reason: String) {
        let allowed = false;
        self.resolve(allowed, reason);
    }
    pub fn answered(&self) -> bool {
        let answered = self.decision_value().is_some();
        answered
    }
    pub async fn wait(&self) -> (bool, String) {
        loop {
            if let Some(result) = self.decision_value() {
                return result;
            }
            self.decision.notify.notified().await;
        }
    }
    fn decision_value(&self) -> Option<(bool, String)> {
        let result = self.decision.result.lock();
        result.map(|value| value.clone()).unwrap_or(None)
    }
    fn resolve(&self, allowed: bool, reason: String) {
        if let Ok(mut result) = self.decision.result.lock() {
            if result.is_none() {
                *result = Some((allowed, reason));
                self.decision.notify.notify_waiters();
            }
        }
    }
}
impl PartialEq for Approval {
    fn eq(&self, other: &Self) -> bool {
        self.tool == other.tool && self.input == other.input
    }
}
impl Eq for Approval {}
