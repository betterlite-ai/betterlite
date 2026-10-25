use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaudeCodeOptions {
    pub config: BTreeMap<String, serde_json::Value>,
    pub env: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodexOptions {
    pub reasoning_effort: Option<String>,
    pub web_search: bool,
    pub config: BTreeMap<String, serde_json::Value>,
    pub env: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OpenCodeOptions {
    pub agent: String,
    pub config: BTreeMap<String, serde_json::Value>,
    pub env: BTreeMap<String, String>,
}
impl Default for OpenCodeOptions {
    fn default() -> Self {
        Self {
            agent: "build".to_owned(),
            config: BTreeMap::new(),
            env: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeepAgentsOptions {
    pub subagents: Vec<serde_json::Value>,
    pub recursion_limit: Option<u32>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HarnessOptions {
    ClaudeCode(ClaudeCodeOptions),
    Codex(CodexOptions),
    OpenCode(OpenCodeOptions),
    DeepAgents(DeepAgentsOptions),
}
