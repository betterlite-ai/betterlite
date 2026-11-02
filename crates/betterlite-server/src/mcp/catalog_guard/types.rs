use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;

#[derive(Debug, Clone, PartialEq)]
pub struct MCPTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: BTreeMap<String, JsonValue>,
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
