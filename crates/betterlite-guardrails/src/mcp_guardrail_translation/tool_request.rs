use std::collections::BTreeMap;

use super::json_value::McpValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_tool_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_input_schema: Option<McpValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_arguments: Option<McpValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<McpValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_arguments: Option<McpValue>,
    // the synthetic MCP request carries the proxy's own keys, which pass through untouched
    #[serde(flatten)]
    pub extra: BTreeMap<String, McpValue>,
}
