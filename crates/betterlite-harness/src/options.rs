use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::JsonObject;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClaudeCodeOptions {
    pub config: JsonObject,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CodexOptions {
    pub reasoning_effort: Option<ReasoningEffort>,
    pub web_search: bool,
    pub config: JsonObject,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCodeOptions {
    pub agent: String,
    pub config: JsonObject,
    pub env: BTreeMap<String, String>,
}

impl Default for OpenCodeOptions {
    fn default() -> Self {
        Self {
            agent: "build".to_owned(),
            config: JsonObject::new(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeepAgentsOptions {
    pub subagents: Vec<JsonObject>,
    pub recursion_limit: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessOptions {
    ClaudeCode(ClaudeCodeOptions),
    Codex(CodexOptions),
    OpenCode(OpenCodeOptions),
    DeepAgents(DeepAgentsOptions),
}

impl HarnessOptions {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ClaudeCode(_) => ClaudeCodeOptions::NAME,
            Self::Codex(_) => CodexOptions::NAME,
            Self::OpenCode(_) => OpenCodeOptions::NAME,
            Self::DeepAgents(_) => DeepAgentsOptions::NAME,
        }
    }
}

pub trait OptionsKind: Clone + Default {
    const NAME: &'static str;

    fn pick(options: &HarnessOptions) -> Option<&Self>;
}

impl OptionsKind for ClaudeCodeOptions {
    const NAME: &'static str = "ClaudeCodeOptions";

    fn pick(options: &HarnessOptions) -> Option<&Self> {
        match options {
            HarnessOptions::ClaudeCode(picked) => Some(picked),
            HarnessOptions::Codex(_)
            | HarnessOptions::OpenCode(_)
            | HarnessOptions::DeepAgents(_) => None,
        }
    }
}

impl OptionsKind for CodexOptions {
    const NAME: &'static str = "CodexOptions";

    fn pick(options: &HarnessOptions) -> Option<&Self> {
        match options {
            HarnessOptions::Codex(picked) => Some(picked),
            HarnessOptions::ClaudeCode(_)
            | HarnessOptions::OpenCode(_)
            | HarnessOptions::DeepAgents(_) => None,
        }
    }
}

impl OptionsKind for OpenCodeOptions {
    const NAME: &'static str = "OpenCodeOptions";

    fn pick(options: &HarnessOptions) -> Option<&Self> {
        match options {
            HarnessOptions::OpenCode(picked) => Some(picked),
            HarnessOptions::ClaudeCode(_)
            | HarnessOptions::Codex(_)
            | HarnessOptions::DeepAgents(_) => None,
        }
    }
}

impl OptionsKind for DeepAgentsOptions {
    const NAME: &'static str = "DeepAgentsOptions";

    fn pick(options: &HarnessOptions) -> Option<&Self> {
        match options {
            HarnessOptions::DeepAgents(picked) => Some(picked),
            HarnessOptions::ClaudeCode(_)
            | HarnessOptions::Codex(_)
            | HarnessOptions::OpenCode(_) => None,
        }
    }
}
