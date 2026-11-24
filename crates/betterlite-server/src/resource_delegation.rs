pub mod admin_grant;
pub mod denial;
pub mod gates;
pub mod grandfather;
pub mod requested;
pub mod server_validation;
pub mod team_read;

mod tests;

use betterlite_auth::auth_model_patterns::access_groups::AccessGroupLookup;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::registry::servers::SharedRegistry;
use betterlite_mcp::server_record::MCPServer;
use serde::Deserialize;

// The permission fields a key or team can carry for MCP servers, toolsets and search tools.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ObjectPermissionGrants {
    #[serde(default)]
    pub mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_access_groups: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_tool_permissions: Option<JsonData>,
    #[serde(default)]
    pub mcp_toolsets: Option<Vec<String>>,
    #[serde(default)]
    pub search_tools: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct TeamGrants {
    pub team_id: String,
    pub access_group_ids: Vec<String>,
    pub object_permission: Option<ObjectPermissionGrants>,
}

// Everything the team reads need: the optional database, the MCP registry records, the access-group row port and the unified access-group lookup.
pub struct GrantReads<'a, Rows> {
    pub client: Option<PrismaClient<'a>>,
    pub registry: &'a SharedRegistry,
    pub rows: Option<&'a Rows>,
    pub unified: Option<&'a dyn AccessGroupLookup>,
}

pub(crate) async fn registry_servers(registry: &SharedRegistry) -> Vec<MCPServer> {
    let guard = registry.read().await;
    guard.get_registry().into_iter().cloned().collect()
}

// Python renders sorted(...) inside the denial detail, so the text keeps its quoted, bracketed list shape.
#[must_use]
pub(crate) fn quoted_list(values: impl IntoIterator<Item = &String>) -> String {
    let mut ordered: Vec<&str> = values.into_iter().map(String::as_str).collect();
    ordered.sort_unstable();
    let quoted: Vec<String> = ordered.iter().map(|value| format!("'{value}'")).collect();
    format!("[{}]", quoted.join(", "))
}
