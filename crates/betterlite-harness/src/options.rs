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

macro_rules! options_kind {
    ($options:ident, $variant:ident, [$($other:ident),+]) => {
        impl OptionsKind for $options {
            const NAME: &'static str = stringify!($options);

            fn pick(options: &HarnessOptions) -> Option<&Self> {
                match options {
                    HarnessOptions::$variant(picked) => Some(picked),
                    $(HarnessOptions::$other(_))|+ => None,
                }
            }
        }
    };
}

options_kind!(ClaudeCodeOptions, ClaudeCode, [Codex, OpenCode, DeepAgents]);
options_kind!(CodexOptions, Codex, [ClaudeCode, OpenCode, DeepAgents]);
options_kind!(OpenCodeOptions, OpenCode, [ClaudeCode, Codex, DeepAgents]);
options_kind!(DeepAgentsOptions, DeepAgents, [ClaudeCode, Codex, OpenCode]);
