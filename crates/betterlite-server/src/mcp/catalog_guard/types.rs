use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MCPTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    #[serde(alias = "input_schema")]
    pub input_schema: BTreeMap<String, JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(alias = "output_schema")]
    pub output_schema: Option<BTreeMap<String, JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<BTreeMap<String, JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<BTreeMap<String, JsonValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icons: Option<Vec<JsonValue>>,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, JsonValue>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MCPServer {
    pub server_id: String,
    pub name: String,
    pub alias: Option<String>,
    pub server_name: Option<String>,
    pub short_prefix: Option<String>,
    pub extra_headers: Vec<String>,
    pub tool_name_to_description: BTreeMap<String, String>,
}

impl MCPServer {
    #[must_use]
    pub fn rate_limit_name(&self) -> &str {
        if let Some(alias) = present_name(self.alias.as_ref()) {
            return alias;
        }
        if let Some(server_name) = present_name(self.server_name.as_ref()) {
            return server_name;
        }
        &self.name
    }
}

const fn present_name(value: Option<&String>) -> Option<&str> {
    match value {
        Some(name) if !name.is_empty() => Some(name.as_str()),
        _ => None,
    }
}
