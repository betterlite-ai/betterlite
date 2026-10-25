mod tests;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::errors::HarnessError;

#[expect(
    clippy::disallowed_types,
    reason = "tool input, runtime config, subagents and history are JSON objects whose shape each agent runtime defines"
)]
pub type JsonObject = serde_json::Map<String, serde_json::Value>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
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
    pub const MEMBERS: [Self; 4] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::OpenCode,
        Self::DeepAgents,
    ];

    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::DeepAgents => "deepagents",
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "CLAUDE_CODE",
            Self::Codex => "CODEX",
            Self::OpenCode => "OPENCODE",
            Self::DeepAgents => "DEEPAGENTS",
        }
    }
}

// Rust callers can only pass a Harness member, so what is left is rejecting a string with a hint
#[must_use]
pub fn require_harness(given: &str) -> HarnessError {
    let normalized = given.trim().to_lowercase().replace('-', "_");
    let hint = Harness::MEMBERS
        .into_iter()
        .find(|member| member.value() == normalized);
    hint.map_or_else(
        || HarnessError::NotAHarness {
            given: given.to_owned(),
        },
        |harness| HarnessError::HarnessHint {
            given: given.to_owned(),
            hint: harness,
        },
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Done,
    MaxTurns,
    Timeout,
    Cancelled,
    RuntimeError,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    ReadOnly,
    Ask,
    Edit,
    #[default]
    Full,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub calls: u64,
}

impl Usage {
    pub const fn total_tokens(&self) -> Result<u64, HarnessError> {
        match self.input_tokens.checked_add(self.output_tokens) {
            Some(total) => Ok(total),
            None => Err(HarnessError::TokenOverflow { usage: *self }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    StructuredOutput,
    ToolApproval,
    ToolFiltering,
    History,
    CustomTools,
    Skills,
    Resume,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub supported: BTreeSet<Capability>,
    pub permission_modes: BTreeSet<PermissionMode>,
}
