use serde::Deserialize;

use crate::tool_records::records::{ToolInputPolicy, ToolOutputPolicy};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ToolDiscoveryItem {
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub key_hash: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolPolicyUpdate<'name> {
    pub tool_name: &'name str,
    pub updated_by: Option<&'name str>,
    pub input_policy: Option<ToolInputPolicy>,
    pub output_policy: Option<ToolOutputPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BlockedToolRow {
    pub object_permission_id: String,
    #[serde(default)]
    pub blocked_tools: Option<Vec<String>>,
}
