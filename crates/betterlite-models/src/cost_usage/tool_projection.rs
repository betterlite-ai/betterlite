use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::cost_usage::CostUsage;
use crate::cost_usage::server_tool_projection::ServerToolUse;
use crate::decisions::Json;

// billing's Usage view: B1668's CostUsage plus the server-tool counters LiteLLM keeps beside it
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ToolUsage {
    pub cost: CostUsage,
    pub server_tool_use: Option<ServerToolUse>,
    #[serde(rename = "server_side_tool_usage_details")]
    pub server_side_details: Option<IndexMap<String, Json>>,
}

impl ToolUsage {
    #[must_use]
    pub fn with_server_tool_use(&self, server_tool_use: ServerToolUse) -> Self {
        Self {
            cost: self.cost.clone(),
            server_tool_use: Some(server_tool_use),
            server_side_details: self.server_side_details.clone(),
        }
    }
}

mod tests;
