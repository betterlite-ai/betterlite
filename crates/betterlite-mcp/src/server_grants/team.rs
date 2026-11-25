use betterlite_auth::ui_session_identity::UI_SESSION_TOKEN_TEAM_ID;
use indexmap::IndexSet;
use tracing::warn;

use super::auth::GrantAuth;
use super::error::ServerGrantsError;
use super::{
    ALL_PROXY_MCP_SERVERS, GrantReads, GrantsError, RegistryGrantReads as _, ServerGrants,
};
use crate::auth::object_permission_hydration::grants::toolset_tool_permissions;
use crate::auth::object_permission_hydration::reads::{
    ObjectPermission, ObjectPermissionReads as _,
};

// _team_ids_for_mcp_grant: only the caller's own team_id counts, never the other teams of its user
#[must_use]
pub fn team_id_for_grant(auth: Option<&GrantAuth>) -> Option<&str> {
    let team_id = auth?.mcp.team_id.as_deref()?;
    if team_id.is_empty() || team_id == UI_SESSION_TOKEN_TEAM_ID {
        return None;
    }
    Some(team_id)
}

impl<Reads> ServerGrants<Reads>
where
    Reads: GrantReads,
{
    // _get_allowed_mcp_servers_for_team
    pub async fn team_servers(
        &self,
        auth: Option<&GrantAuth>,
    ) -> Result<Vec<String>, GrantsError<Reads>> {
        let Some(team_id) = team_id_for_grant(auth) else {
            return Ok(Vec::new());
        };
        let fresh = auth.is_some_and(|record| record.mcp.requires_fresh_policy);
        self.single_team_servers(team_id, fresh).await
    }

    // _allowed_mcp_servers_for_single_team: an unloadable entitlement, or any fault under the fresh policy, denies
    pub async fn single_team_servers(
        &self,
        team_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<Vec<String>, GrantsError<Reads>> {
        if team_id.is_empty()
            || team_id == UI_SESSION_TOKEN_TEAM_ID
            || !self.reads.permissions().has_database()
        {
            return Ok(Vec::new());
        }
        match self.team_grant(team_id, requires_fresh_policy).await {
            Ok(servers) => Ok(servers),
            Err(error) => {
                if requires_fresh_policy || matches!(error, ServerGrantsError::Unloadable(_)) {
                    return Err(error);
                }
                warn!("Failed to get allowed MCP servers for team: {error}");
                Ok(Vec::new())
            }
        }
    }

    async fn team_grant(
        &self,
        team_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<Vec<String>, GrantsError<Reads>> {
        let found = self.reads.team(team_id, requires_fresh_policy).await?;
        let Some(team) = found else {
            return Err(ServerGrantsError::TeamMissing {
                team_id: team_id.to_owned(),
            });
        };
        // a keyless admitted subject has no key whose policy gate would catch a blocked team
        if team.blocked {
            return Ok(Vec::new());
        }
        let group_servers = self
            .reads
            .access_group_server_ids(&team.access_group_ids, requires_fresh_policy)
            .await?;
        self.team_granted_servers(
            team.object_permission.as_ref(),
            group_servers,
            requires_fresh_policy,
        )
        .await
    }

    // _team_granted_servers: all-proxy-mcpservers is the whole registry and nothing the team adds besides
    pub async fn team_granted_servers(
        &self,
        permission: Option<&ObjectPermission>,
        team_access_group_servers: Vec<String>,
        requires_fresh_policy: bool,
    ) -> Result<Vec<String>, GrantsError<Reads>> {
        let Some(row) = permission else {
            let distinct: IndexSet<String> = team_access_group_servers.into_iter().collect();
            return Ok(distinct.into_iter().collect());
        };
        let mcp_servers = row.mcp_servers.0.as_deref().unwrap_or_default();
        let registry = self.reads.registry();
        if mcp_servers
            .iter()
            .any(|server| server.as_str() == ALL_PROXY_MCP_SERVERS)
        {
            return Ok(registry.registry_server_ids().await);
        }
        let access_groups = row.mcp_access_groups.0.as_deref().unwrap_or_default();
        let legacy_servers = self
            .reads
            .servers_from_access_groups(access_groups, requires_fresh_policy)
            .await?;
        let toolset_grants =
            toolset_tool_permissions(self.reads.permissions(), Some(row), requires_fresh_policy)
                .await?;
        let mut servers: IndexSet<String> = registry
            .expand_permission_list(mcp_servers)
            .await
            .into_iter()
            .collect();
        servers.extend(legacy_servers);
        let tool_grants = registry
            .expand_tool_permissions(row.mcp_tool_permissions.as_ref())
            .await;
        servers.extend(tool_grants.into_keys());
        servers.extend(toolset_grants.into_keys());
        servers.extend(team_access_group_servers);
        Ok(servers.into_iter().collect())
    }
}
