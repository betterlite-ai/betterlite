mod reads;
mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;

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
    permission: ObjectPermission,
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
    permission: Option<ObjectPermission>,
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
    let auth = GrantAuth {
        mcp: McpAuth {
            user_id: None,
            team_id: None,
            org_id: None,
            end_user_id: None,
            agent_id: None,
            object_permission_id: None,
            object_permission: Some(call.permission.clone()),
            requires_fresh_policy: false,
        },
        access_group_ids: call.access_group_ids.clone(),
    };
    key_servers_and_groups(call, &auth).await
}

async fn key_servers_and_groups(call: &KeyCall, auth: &GrantAuth) -> Vec<String> {
    let reads = case_reads(
        &call.servers,
        &call.access_groups,
        &call.toolset_grants,
        None,
    );
    let grants = ServerGrants { reads };
    let key_servers = grants.key_servers(Some(auth)).await;
    if key_servers
        .iter()
        .any(|server| server.as_str() == NO_MCP_SERVERS)
    {
        return Vec::new();
    }
    let mut server_ids: BTreeSet<String> = key_servers.into_iter().collect();
    server_ids.extend(grants.key_access_group_servers(Some(auth)).await);
    server_ids.into_iter().collect()
}

// a keyless caller pinned to one team: the resolver's base is the team list
async fn team_answer(call: &TeamCall) -> Vec<String> {
    let team = TeamRecord {
        blocked: call.blocked,
        access_group_ids: call.team_access_group_ids.clone(),
        object_permission: call.permission.clone(),
    };
    let reads = case_reads(
        &call.servers,
        &call.access_groups,
        &call.toolset_grants,
        Some(team),
    );
    // the case pins a normal team id, and a raising team level answers an empty list anyway
    let mut server_ids = ServerGrants { reads }
        .single_team_servers(&call.team_id, false)
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
