pub mod usage;

mod tests;

use std::collections::BTreeSet;

use crate::deepagents::transformation::message::{ToolCall, ToolResult};
use crate::types::JsonObject;

pub const TOOL_FILTER_NAME: &str = "LiteLLMHarnessToolFilter";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelTool {
    pub name: String,
    pub description: Option<String>,
    pub parameters: JsonObject,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolFilterMiddleware {
    pub blocked: BTreeSet<String>,
}

impl ToolFilterMiddleware {
    #[must_use]
    pub fn filter_tools(&self, tools: Vec<ModelTool>) -> Vec<ModelTool> {
        tools
            .into_iter()
            .filter(|tool| !self.blocked.contains(&tool.name))
            .collect()
    }

    #[must_use]
    pub fn refusal(&self, call: &ToolCall) -> Option<ToolResult> {
        if self.blocked.contains(&call.name) {
            return Some(ToolResult {
                id: call.id.clone(),
                output: format!("Error: {} is disabled for this session.", call.name),
                is_error: true,
            });
        }
        None
    }
}
