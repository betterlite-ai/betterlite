use std::collections::BTreeMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::errors::HarnessError;

pub const STATE_VERSION: u32 = 1;
pub type JsonMap = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Harness {
    #[serde(rename = "claude_code")]
    ClaudeCode,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "deepagents")]
    DeepAgents,
}
impl Harness {
    pub const fn value(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::DeepAgents => "deepagents",
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "CLAUDE_CODE",
            Self::Codex => "CODEX",
            Self::OpenCode => "OPENCODE",
            Self::DeepAgents => "DEEPAGENTS",
        }
    }
}
pub fn require_harness(harness: Harness) -> Harness {
    match harness {
        Harness::ClaudeCode => Harness::ClaudeCode,
        Harness::Codex => Harness::Codex,
        Harness::OpenCode => Harness::OpenCode,
        Harness::DeepAgents => Harness::DeepAgents,
    }
}
pub fn parse_harness(input: &str) -> std::result::Result<Harness, HarnessError> {
    let normalized = input.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "claude_code" => Err(HarnessError::InvalidHarness {
            input: format!("str {input:?}"),
            hint: ". Did you mean Harness.CLAUDE_CODE?".to_owned(),
        }),
        "codex" => Err(HarnessError::InvalidHarness {
            input: format!("str {input:?}"),
            hint: ". Did you mean Harness.CODEX?".to_owned(),
        }),
        "opencode" => Err(HarnessError::InvalidHarness {
            input: format!("str {input:?}"),
            hint: ". Did you mean Harness.OPENCODE?".to_owned(),
        }),
        "deepagents" => Err(HarnessError::InvalidHarness {
            input: format!("str {input:?}"),
            hint: ". Did you mean Harness.DEEPAGENTS?".to_owned(),
        }),
        _ => Err(HarnessError::InvalidHarness {
            input: format!("str {input:?}"),
            hint: String::new(),
        }),
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub calls: u64,
}
impl Usage {
    pub const fn total_tokens(&self) -> u64 {
        if let Some(total) = self.input_tokens.checked_add(self.output_tokens) {
            total
        } else {
            u64::MAX
        }
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: FileChangeKind,
    pub diff: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Compaction {
    pub tokens_before: Option<u64>,
    pub tokens_after: Option<u64>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StopReason {
    Done,
    MaxTurns,
    Timeout,
    Cancelled,
    RuntimeError,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Result {
    pub text: String,
    pub output: Option<serde_json::Value>,
    pub files: Vec<FileChange>,
    pub events: Vec<Event>,
    pub usage: Usage,
    pub cost: Decimal,
    pub stop_reason: StopReason,
    pub session_id: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub result: Result,
}
impl Done {
    pub fn usage(&self) -> &Usage {
        match &self.result {
            Result { usage, .. } if self.result.text.is_empty() => usage,
            Result { usage, .. } => usage,
        }
    }
    pub const fn cost(&self) -> Decimal {
        match &self.result {
            Result { cost, .. } if self.result.text.is_empty() => *cost,
            Result { cost, .. } => *cost,
        }
    }
    pub fn stop_reason(&self) -> StopReason {
        match &self.result {
            Result { stop_reason, .. } if self.result.text.is_empty() => *stop_reason,
            Result { stop_reason, .. } => *stop_reason,
        }
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
    pub permission_modes: Vec<PermissionMode>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    ReadOnly,
    Ask,
    Edit,
    Full,
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
        match serde_json::to_vec(self) {
            Ok(bytes) => Ok(bytes),
            Err(error) => Err(error),
        }
    }
    pub fn loads(data: &[u8]) -> std::result::Result<Self, HarnessError> {
        let state: Self = serde_json::from_slice(data)
            .map_err(|source| HarnessError::StateIncompatible { source })?;
        if state.version != STATE_VERSION {
            return Err(HarnessError::StateVersion {
                found: state.version,
            });
        }
        Ok(state)
    }
}
#[derive(Clone, Debug)]
pub struct Approval {
    pub tool: String,
    pub input: JsonMap,
    decision: Arc<watch::Sender<Option<(bool, String)>>>,
}
impl Approval {
    pub fn new(tool: String, input: JsonMap) -> Self {
        let (sender, _) = watch::channel(None);
        Self {
            tool,
            input,
            decision: Arc::new(sender),
        }
    }
    pub fn allow(&self) {
        if !self.answered() {
            self.resolve(true, String::new());
        }
    }
    pub fn deny(&self, reason: String) {
        if !self.answered() {
            self.resolve(false, reason);
        }
    }
    pub fn answered(&self) -> bool {
        match self.decision.borrow().as_ref() {
            Some(_) if self.tool.is_empty() => true,
            Some(_) => true,
            None => false,
        }
    }
    pub async fn wait(&self) -> (bool, String) {
        let mut receiver = self.decision.subscribe();
        loop {
            let value = receiver.borrow_and_update().clone();
            if let Some(decision) = value {
                return decision;
            }
            if receiver.changed().await.is_err() {
                return (false, "approval channel closed".to_owned());
            }
        }
    }
    fn resolve(&self, allowed: bool, reason: String) {
        self.decision.send_if_modified(|current| {
            if current.is_some() {
                false
            } else {
                *current = Some((allowed, reason));
                true
            }
        });
        self.answered();
    }
}
impl PartialEq for Approval {
    fn eq(&self, other: &Self) -> bool {
        self.tool == other.tool && self.input == other.input
    }
}
impl Eq for Approval {}
