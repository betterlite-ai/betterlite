use std::collections::BTreeMap;
use std::future::Future;

use betterlite_auth::ui_session_identity::UI_SESSION_TOKEN_TEAM_ID;
use indexmap::IndexSet;
use tracing::warn;

use crate::auth::object_permission_hydration::grants::toolset_tool_permissions;
use crate::auth::object_permission_hydration::reads::{ObjectPermission, ObjectPermissionReads};

use self::auth::GrantAuth;
use self::error::{GrantReadError, ServerGrantsError};
use self::key::{KeyLevel, key_level};
use self::team::{team_grant, team_id_for_grant};
use self::user::distinct_team_ids;

pub mod auth;
pub mod db_reads;
pub mod error;
pub mod key;
pub mod team;
pub mod user;

mod tests;

// SpecialMCPServerNames.no_mcp_servers: a key that lists it opts out of every MCP server
pub const NO_MCP_SERVERS: &str = "no-mcp-servers";
// SpecialMCPServerName.all_proxy_servers: special only on a team, where it grants the whole registry
pub const ALL_PROXY_MCP_SERVERS: &str = "all-proxy-mcpservers";

pub type ToolGrants = BTreeMap<String, Vec<String>>;

pub type GrantsError<Reads> =
    ServerGrantsError<<<Reads as GrantReads>::Permissions as ObjectPermissionReads>::Error>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRecord {
    pub blocked: bool,
    pub access_group_ids: Vec<String>,
    pub object_permission: Option<ObjectPermission>,
}

// the server manager's registry expansion; its own task implements this over the live registry
pub trait RegistryGrantReads: Send + Sync {
    // names and aliases become every matching server id, an id or an unknown entry passes through
    fn expand_permission_list(
        &self,
        identifiers: &[String],
    ) -> impl Future<Output = Vec<String>> + Send;

    fn expand_tool_permissions(
        &self,
        tool_permissions: Option<&ToolGrants>,
    ) -> impl Future<Output = ToolGrants> + Send;

    fn registry_server_ids(&self) -> impl Future<Output = Vec<String>> + Send;
}

pub trait GrantReads: Send + Sync {
    type Permissions: ObjectPermissionReads;
    type Registry: RegistryGrantReads;

    // B4668's reads: the hydrated key permission, has_database and the toolset resolver
    fn permissions(&self) -> &Self::Permissions;

    fn registry(&self) -> &Self::Registry;

    // get_team_object: None is a team row the database does not have
    fn team(
        &self,
        team_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Option<TeamRecord>, GrantReadError>> + Send;

    // get_user_object without upsert: None is a user the database does not have
    fn user_team_ids(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Option<Vec<String>>, GrantReadError>> + Send;

    // _get_mcp_server_ids_from_access_groups over the unified access groups
    fn access_group_server_ids(
        &self,
        access_group_ids: &[String],
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Vec<String>, GrantReadError>> + Send;

    // B10078's servers_from_access_groups over the tag-style mcp_access_groups
    fn servers_from_access_groups(
        &self,
        access_groups: &[String],
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Vec<String>, GrantReadError>> + Send;
}

pub struct ServerGrants<Reads: GrantReads> {
    pub reads: Reads,
}

// The type's one inherent impl: multiple_inherent_impl keeps each area's logic free in key, team and user.
impl<Reads> ServerGrants<Reads>
where
    Reads: GrantReads,
{
    // _get_allowed_mcp_servers_for_key: an empty key list means the key does not restrict, so LiteLLM fails open on every fault
    pub async fn key_servers(&self, auth: Option<&GrantAuth>) -> Vec<String> {
        let Some(record) = auth else {
            return Vec::new();
        };
        key_level(self, record, KeyLevel::Ceiling).await
    }

    // _get_key_access_group_mcp_server_extras: attaching a unified group to the key is itself the grant
    pub async fn key_access_group_servers(&self, auth: Option<&GrantAuth>) -> Vec<String> {
        let Some(record) = auth else {
            return Vec::new();
        };
        key_level(self, record, KeyLevel::AccessGroups).await
    }

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
        match team_grant(self, team_id, requires_fresh_policy).await {
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

    // _resolve_user_team_ids: a lookup blip narrows to the caller's direct grants instead of raising
    pub async fn user_team_ids(&self, user_id: &str, requires_fresh_policy: bool) -> Vec<String> {
        if !self.reads.permissions().has_database() {
            return Vec::new();
        }
        let teams = match self
            .reads
            .user_team_ids(user_id, requires_fresh_policy)
            .await
        {
            Ok(found) => found.unwrap_or_default(),
            Err(error) => {
                warn!("Failed to resolve user teams for MCP grant: {error}");
                return Vec::new();
            }
        };
        distinct_team_ids(teams)
    }
}
