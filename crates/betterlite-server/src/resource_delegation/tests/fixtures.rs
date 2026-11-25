pub(crate) mod database;
pub(crate) mod fakes;
pub(crate) mod logs;

use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_auth::auth_model_patterns::access_groups::AccessGroupLookup;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamFields;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use betterlite_mcp::server_record::MCPServer;
use tokio::sync::RwLock;

use crate::resource_delegation::admin_grant::AdminGrantError;
use crate::resource_delegation::denial::Denial;
use crate::resource_delegation::gates::GateError;
use crate::resource_delegation::server_validation::KeyServersError;
use crate::resource_delegation::{GrantReads, ObjectPermissionGrants};

use self::fakes::FakeRows;

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the case value is JSON")
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

pub(crate) fn string_set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

pub(crate) fn registry_server(text: &str) -> MCPServer {
    serde_json::from_str(text).expect("the registry record is an MCP server")
}

pub(crate) fn registry(config: Vec<MCPServer>, db: Vec<MCPServer>) -> SharedRegistry {
    let settings = RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: false,
        internal_ip_ranges: None,
    };
    Arc::new(RwLock::new(ServerRegistry::new(settings, config, db)))
}

pub(crate) fn server_grants(
    servers: Option<&[&str]>,
    access_groups: Option<&[&str]>,
    tool_permissions: Option<JsonData>,
) -> ObjectPermissionGrants {
    ObjectPermissionGrants {
        mcp_servers: servers.map(strings).unwrap_or_default(),
        mcp_access_groups: access_groups.map(strings).unwrap_or_default(),
        mcp_tool_permissions: tool_permissions.unwrap_or(JsonData::Null),
        mcp_toolsets: Vec::new(),
        search_tools: Vec::new(),
    }
}

pub(crate) fn ceiling_grants(
    toolsets: Option<&[&str]>,
    search_tools: Option<&[&str]>,
) -> ObjectPermissionGrants {
    ObjectPermissionGrants {
        mcp_servers: Vec::new(),
        mcp_access_groups: Vec::new(),
        mcp_tool_permissions: JsonData::Null,
        mcp_toolsets: toolsets.map(strings).unwrap_or_default(),
        search_tools: search_tools.map(strings).unwrap_or_default(),
    }
}

pub(crate) fn tool_permission_grants(raw: JsonData) -> ObjectPermissionGrants {
    ObjectPermissionGrants {
        mcp_servers: Vec::new(),
        mcp_access_groups: Vec::new(),
        mcp_tool_permissions: raw,
        mcp_toolsets: Vec::new(),
        search_tools: Vec::new(),
    }
}

pub(crate) fn team_fields(
    team_id: &str,
    access_group_ids: &[&str],
    permission: Option<ObjectPermissionGrants>,
) -> TeamFields {
    let mut row = serde_json::json!({
        "team_id": team_id,
        "access_group_ids": access_group_ids,
    });
    if let Some(grants) = permission {
        row["object_permission"] = serde_json::json!({
            "object_permission_id": "team-permission",
            "mcp_servers": grants.mcp_servers,
            "mcp_access_groups": grants.mcp_access_groups,
            "mcp_tool_permissions": grants.mcp_tool_permissions,
            "mcp_toolsets": grants.mcp_toolsets,
            "search_tools": grants.search_tools,
        });
    }
    serde_json::from_value(row).expect("the team row")
}

pub(crate) fn reads<'reads>(
    registry: &'reads SharedRegistry,
    client: Option<PrismaClient<'reads>>,
    rows: &'reads FakeRows,
    unified: Option<&'reads dyn AccessGroupLookup>,
) -> GrantReads<'reads, FakeRows> {
    GrantReads {
        client,
        registry,
        rows: Some(rows),
        unified,
    }
}

pub(crate) fn denied_detail<RowError>(error: KeyServersError<RowError>) -> String
where
    RowError: StdError + Send + Sync + 'static,
{
    match error {
        KeyServersError::Denied(Denial { detail }) => detail,
        KeyServersError::Requested(cause) => panic!("a request error, not a denial: {cause}"),
        KeyServersError::TeamRead(cause) => panic!("a team read error, not a denial: {cause}"),
        KeyServersError::Resolve(cause) => panic!("a resolve error, not a denial: {cause}"),
        KeyServersError::Cleanup(cause) => panic!("a cleanup error, not a denial: {cause}"),
        KeyServersError::Grandfather(cause) => panic!("a grandfather error, not a denial: {cause}"),
    }
}

pub(crate) fn gate_detail(error: GateError) -> String {
    match error {
        GateError::Denied(Denial { detail }) => detail,
        GateError::Requested(cause) => panic!("a request error, not a denial: {cause}"),
    }
}

pub(crate) fn admin_detail(error: AdminGrantError) -> String {
    match error {
        AdminGrantError::Denied(Denial { detail }) => detail,
        AdminGrantError::Existing(cause) => panic!("a stored read error, not a denial: {cause}"),
    }
}
