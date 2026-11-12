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

mod tests;
