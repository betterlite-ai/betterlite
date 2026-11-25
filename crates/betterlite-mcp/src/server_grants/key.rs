use indexmap::IndexSet;
use tracing::warn;

use super::auth::GrantAuth;
use super::error::ServerGrantsError;
use super::{GrantReads, GrantsError, NO_MCP_SERVERS, RegistryGrantReads as _, ServerGrants};
use crate::auth::object_permission_hydration::principal::key_object_permission_hydrated;
use crate::auth::object_permission_hydration::reads::{
    ObjectPermission, ObjectPermissionReads as _,
};

// the two key level grants differ only in where they read and in the warning they leave
#[derive(Clone, Copy, Debug)]
pub(super) enum KeyLevel {
    Ceiling,
    AccessGroups,
}

pub(super) async fn key_level<Reads>(
    grants: &ServerGrants<Reads>,
    auth: &GrantAuth,
    level: KeyLevel,
) -> Vec<String>
where
    Reads: GrantReads,
{
    let granted = match level {
        KeyLevel::Ceiling => key_grant(grants, auth).await,
        KeyLevel::AccessGroups => {
            key_access_group_grant(
                grants,
                &auth.access_group_ids,
                auth.mcp.requires_fresh_policy,
            )
            .await
        }
    };
    match granted {
        Ok(servers) => servers,
        Err(error) => {
            let warning = match level {
                KeyLevel::Ceiling => "Failed to get allowed MCP servers for key",
                KeyLevel::AccessGroups => "Failed to get key access group MCP server grants",
            };
            warn!("{warning}: {error}");
            Vec::new()
        }
    }
}

// _get_allowed_mcp_servers_for_key: an empty key list means the key does not restrict, so LiteLLM fails open on every fault
async fn key_grant<Reads>(
    grants: &ServerGrants<Reads>,
    auth: &GrantAuth,
) -> Result<Vec<String>, GrantsError<Reads>>
where
    Reads: GrantReads,
{
    let permissions = grants.reads.permissions();
    let hydrated = key_object_permission_hydrated(permissions, &auth.mcp)
        .await
        .map_err(|source| ServerGrantsError::Permission { source })?;
    let Some(permission) = hydrated else {
        return Ok(Vec::new());
    };
    let mcp_servers = permission.mcp_servers.0.as_deref().unwrap_or_default();
    // the opt-out is read before any expansion, so the caller can stop before inheriting the team
    if mcp_servers
        .iter()
        .any(|server| server.as_str() == NO_MCP_SERVERS)
    {
        return Ok(Vec::from([NO_MCP_SERVERS.to_owned()]));
    }
    key_permission_servers(grants, &permission, auth.mcp.requires_fresh_policy).await
}

// the four sources a hydrated key permission adds: its servers, tag groups, tool permissions and toolsets
async fn key_permission_servers<Reads>(
    grants: &ServerGrants<Reads>,
    permission: &ObjectPermission,
    requires_fresh_policy: bool,
) -> Result<Vec<String>, GrantsError<Reads>>
where
    Reads: GrantReads,
{
    let registry = grants.reads.registry();
    let mut servers: IndexSet<String> = registry
        .expand_permission_list(permission.mcp_servers.0.as_deref().unwrap_or_default())
        .await
        .into_iter()
        .collect();
    let access_groups = permission
        .mcp_access_groups
        .0
        .as_deref()
        .unwrap_or_default();
    servers.extend(
        grants
            .reads
            .servers_from_access_groups(access_groups, requires_fresh_policy)
            .await?,
    );
    let tool_grants = registry
        .expand_tool_permissions(permission.mcp_tool_permissions.as_ref())
        .await;
    servers.extend(tool_grants.into_keys());
    let toolset_ids = permission.mcp_toolsets.as_deref().unwrap_or_default();
    if !toolset_ids.is_empty() {
        // the resolver itself, not B4668's checked read: an empty answer adds nothing to a key
        let toolset_grants = grants
            .reads
            .permissions()
            .resolve_toolset_grants(toolset_ids, requires_fresh_policy)
            .await
            .map_err(|source| ServerGrantsError::Permission { source })?;
        servers.extend(toolset_grants.into_keys());
    }
    Ok(servers.into_iter().collect())
}

// _get_key_access_group_mcp_server_extras: attaching a unified group to the key is itself the grant
async fn key_access_group_grant<Reads>(
    grants: &ServerGrants<Reads>,
    access_group_ids: &[String],
    requires_fresh_policy: bool,
) -> Result<Vec<String>, GrantsError<Reads>>
where
    Reads: GrantReads,
{
    let server_ids = grants
        .reads
        .access_group_server_ids(access_group_ids, requires_fresh_policy)
        .await?;
    if server_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(grants
        .reads
        .registry()
        .expand_permission_list(&server_ids)
        .await)
}
