use indexmap::IndexMap;
use serde_json::Value;

use crate::api::skills::hook::execution::SkillFiles;

#[derive(Debug, Clone, Default)]
pub struct SkillsRequestData {
    pub model: String,
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
    pub max_tokens: Option<u64>,
    pub skills: RequestMetadata,
}

#[derive(Debug, Clone, Default)]
pub struct RequestMetadata {
    pub code_execution_enabled: bool,
    pub skill_files: IndexMap<String, SkillFiles>,
}
