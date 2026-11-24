pub(crate) mod database;
pub(crate) mod fakes;
pub(crate) mod logs;

use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_auth::auth_model_patterns::access_groups::AccessGroupLookup;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use betterlite_mcp::server_record::MCPServer;
use tokio::sync::RwLock;

use crate::resource_delegation::admin_grant::AdminGrantError;
use crate::resource_delegation::denial::Denial;
use crate::resource_delegation::gates::GateError;
use crate::resource_delegation::server_validation::KeyServersError;
use crate::resource_delegation::{GrantReads, ObjectPermissionGrants, TeamGrants};

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
        mcp_servers: servers.map(strings),
        mcp_access_groups: access_groups.map(strings),
        mcp_tool_permissions: tool_permissions,
        mcp_toolsets: None,
        search_tools: None,
    }
}

pub(crate) fn ceiling_grants(
    toolsets: Option<&[&str]>,
    search_tools: Option<&[&str]>,
) -> ObjectPermissionGrants {
    ObjectPermissionGrants {
        mcp_servers: None,
        mcp_access_groups: None,
        mcp_tool_permissions: None,
        mcp_toolsets: toolsets.map(strings),
        search_tools: search_tools.map(strings),
    }
}

pub(crate) fn tool_permission_grants(raw: JsonData) -> ObjectPermissionGrants {
    ObjectPermissionGrants {
        mcp_servers: None,
        mcp_access_groups: None,
        mcp_tool_permissions: Some(raw),
        mcp_toolsets: None,
        search_tools: None,
    }
}

pub(crate) fn team_grants(
    team_id: &str,
    access_group_ids: &[&str],
    permission: Option<ObjectPermissionGrants>,
) -> TeamGrants {
    TeamGrants {
        team_id: String::from(team_id),
        access_group_ids: strings(access_group_ids),
        object_permission: permission,
    }
}

pub(crate) fn reads<'a>(
    registry: &'a SharedRegistry,
    client: Option<PrismaClient<'a>>,
    rows: &'a FakeRows,
    unified: Option<&'a dyn AccessGroupLookup>,
) -> GrantReads<'a, FakeRows> {
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
        KeyServersError::Requested(error) => panic!("a request error, not a denial: {error}"),
        KeyServersError::TeamRead(error) => panic!("a team read error, not a denial: {error}"),
        KeyServersError::Resolve(error) => panic!("a resolve error, not a denial: {error}"),
        KeyServersError::Cleanup(error) => panic!("a cleanup error, not a denial: {error}"),
        KeyServersError::Grandfather(error) => panic!("a grandfather error, not a denial: {error}"),
    }
}

pub(crate) fn gate_detail(error: GateError) -> String {
    match error {
        GateError::Denied(Denial { detail }) => detail,
        GateError::Requested(error) => panic!("a request error, not a denial: {error}"),
    }
}

pub(crate) fn admin_detail(error: AdminGrantError) -> String {
    match error {
        AdminGrantError::Denied(Denial { detail }) => detail,
        AdminGrantError::Existing(error) => panic!("a stored read error, not a denial: {error}"),
    }
}
