mod reads;
mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_db::budget_permission_rows::PermissionList;
use betterlite_mcp::auth::object_permission_hydration::reads::ObjectPermission;
use betterlite_mcp::auth::object_permission_hydration::record::McpAuth;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_grants::auth::GrantAuth;
use betterlite_mcp::server_grants::{NO_MCP_SERVERS, ServerGrants, TeamRecord, ToolGrants};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;
use tokio::sync::RwLock;

use self::reads::CaseReads;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionRecord {
    #[serde(rename = "mcp_servers")]
    servers: Option<Vec<String>>,
    #[serde(rename = "mcp_access_groups")]
    access_groups: Option<Vec<String>>,
    #[serde(rename = "mcp_tool_permissions")]
    tool_permissions: Option<ToolGrants>,
    #[serde(rename = "mcp_toolsets")]
    toolsets: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccessGroupRecord {
    access_group_id: String,
    access_mcp_server_ids: Vec<String>,
}

// config servers, unified access group rows, and the grants every declared toolset resolves to
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyCall {
    servers: Vec<MCPServer>,
    access_groups: Vec<AccessGroupRecord>,
    toolset_grants: ToolGrants,
    permission: PermissionRecord,
    access_group_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TeamCall {
    servers: Vec<MCPServer>,
    access_groups: Vec<AccessGroupRecord>,
    toolset_grants: ToolGrants,
    team_id: String,
    blocked: bool,
    team_access_group_ids: Vec<String>,
    permission: Option<PermissionRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserTeamsCall {
    user_id: String,
    teams: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Key(KeyCall),
    Team(TeamCall),
    UserTeams(UserTeamsCall),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_time().build()?;
        let server_ids = runtime.block_on(async {
            match input {
                Input::Key(call) => key_answer(call).await,
                Input::Team(call) => team_answer(call).await,
                Input::UserTeams(call) => {
                    let mut reads = case_reads(&[], &[], &ToolGrants::new(), None);
                    reads.user_teams = Some(call.teams.clone());
                    ServerGrants { reads }
                        .user_team_ids(&call.user_id, false)
                        .await
                }
            }
        });
        Ok(json!(server_ids))
    }
}

// with no team the resolver's base is the key list, the opt-out stops it, and the group grants are added on top
async fn key_answer(call: &KeyCall) -> Vec<String> {
    let reads = case_reads(
        &call.servers,
        &call.access_groups,
        &call.toolset_grants,
        None,
    );
    let grants = ServerGrants { reads };
    let mut auth = grant_auth(None, &call.access_group_ids);
    auth.mcp.object_permission = Some(permission_row(&call.permission));
    let key_servers = grants.key_servers(Some(&auth)).await;
    if key_servers
        .iter()
        .any(|server| server.as_str() == NO_MCP_SERVERS)
    {
        return Vec::new();
    }
    let mut server_ids: BTreeSet<String> = key_servers.into_iter().collect();
    server_ids.extend(grants.key_access_group_servers(Some(&auth)).await);
    server_ids.into_iter().collect()
}

// a keyless caller pinned to one team: the resolver's base is the team list
async fn team_answer(call: &TeamCall) -> Vec<String> {
    let team = TeamRecord {
        blocked: call.blocked,
        access_group_ids: call.team_access_group_ids.clone(),
        object_permission: call.permission.as_ref().map(permission_row),
    };
    let reads = case_reads(
        &call.servers,
        &call.access_groups,
        &call.toolset_grants,
        Some(team),
    );
    let auth = grant_auth(Some(call.team_id.as_str()), &[]);
    // get_mcp_server_access answers an empty list when the team level raises
    let mut server_ids = ServerGrants { reads }
        .team_servers(Some(&auth))
        .await
        .unwrap_or_default();
    server_ids.sort();
    server_ids
}

fn case_reads(
    servers: &[MCPServer],
    access_groups: &[AccessGroupRecord],
    toolset_grants: &ToolGrants,
    team: Option<TeamRecord>,
) -> CaseReads {
    let registry = ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: true,
            internal_ip_ranges: None,
        },
        servers.to_vec(),
        Vec::new(),
    );
    CaseReads {
        registry: Arc::new(RwLock::new(registry)),
        access_groups: access_groups.to_vec(),
        toolset_grants: toolset_grants.clone(),
        team,
        user_teams: None,
    }
}

fn grant_auth(team_id: Option<&str>, access_group_ids: &[String]) -> GrantAuth {
    GrantAuth {
        mcp: McpAuth {
            user_id: None,
            team_id: team_id.map(str::to_owned),
            org_id: None,
            end_user_id: None,
            agent_id: None,
            object_permission_id: None,
            object_permission: None,
            requires_fresh_policy: false,
        },
        access_group_ids: access_group_ids.to_vec(),
    }
}

fn permission_row(permission: &PermissionRecord) -> ObjectPermission {
    ObjectPermission {
        object_permission_id: String::from("op-case"),
        mcp_servers: PermissionList(permission.servers.clone()),
        mcp_access_groups: PermissionList(permission.access_groups.clone()),
        mcp_tool_permissions: permission.tool_permissions.clone(),
        vector_stores: PermissionList(None),
        agents: PermissionList(None),
        agent_access_groups: PermissionList(None),
        models: PermissionList(None),
        mcp_toolsets: permission.toolsets.clone(),
        blocked_tools: PermissionList(None),
        search_tools: PermissionList(None),
        mcp_tool_search_enabled: None,
        skills: None,
    }
}
