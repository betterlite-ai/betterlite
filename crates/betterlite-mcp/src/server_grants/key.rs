use indexmap::IndexSet;
use tracing::warn;

use super::auth::GrantAuth;
use super::error::ServerGrantsError;
use super::{GrantReads, GrantsError, NO_MCP_SERVERS, RegistryGrantReads as _, ServerGrants};
use crate::auth::object_permission_hydration::principal::key_object_permission_hydrated;
use crate::auth::object_permission_hydration::reads::ObjectPermissionReads as _;

// the two key level grants differ only in where they read and in the warning they leave
#[derive(Clone, Copy, Debug)]
enum KeyLevel {
    Ceiling,
    AccessGroups,
}

impl<Reads> ServerGrants<Reads>
where
    Reads: GrantReads,
{
    // _get_allowed_mcp_servers_for_key: an empty key list means the key does not restrict, so LiteLLM fails open on every fault
    pub async fn key_servers(&self, auth: Option<&GrantAuth>) -> Vec<String> {
        let Some(record) = auth else {
            return Vec::new();
        };
        self.key_level(record, KeyLevel::Ceiling).await
    }

    // _get_key_access_group_mcp_server_extras: attaching a unified group to the key is itself the grant
    pub async fn key_access_group_servers(&self, auth: Option<&GrantAuth>) -> Vec<String> {
        let Some(record) = auth else {
            return Vec::new();
        };
        self.key_level(record, KeyLevel::AccessGroups).await
    }

    async fn key_level(&self, auth: &GrantAuth, level: KeyLevel) -> Vec<String> {
        let granted = match level {
            KeyLevel::Ceiling => self.key_grant(auth).await,
            KeyLevel::AccessGroups => self.key_access_group_grant(auth).await,
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

    async fn key_grant(&self, auth: &GrantAuth) -> Result<Vec<String>, GrantsError<Reads>> {
        let permissions = self.reads.permissions();
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
        let fresh = auth.mcp.requires_fresh_policy;
        let registry = self.reads.registry();
        let mut servers: IndexSet<String> = registry
            .expand_permission_list(mcp_servers)
            .await
            .into_iter()
            .collect();
        let access_groups = permission
            .mcp_access_groups
            .0
            .as_deref()
            .unwrap_or_default();
        servers.extend(
            self.reads
                .servers_from_access_groups(access_groups, fresh)
                .await?,
        );
        let tool_grants = registry
            .expand_tool_permissions(permission.mcp_tool_permissions.as_ref())
            .await;
        servers.extend(tool_grants.into_keys());
        let toolset_ids = permission.mcp_toolsets.as_deref().unwrap_or_default();
        if !toolset_ids.is_empty() {
            // the resolver itself, not B4668's checked read: an empty answer adds nothing to a key
            let toolset_grants = permissions
                .resolve_toolset_grants(toolset_ids, fresh)
                .await
                .map_err(|source| ServerGrantsError::Permission { source })?;
            servers.extend(toolset_grants.into_keys());
        }
        Ok(servers.into_iter().collect())
    }

    async fn key_access_group_grant(
        &self,
        auth: &GrantAuth,
    ) -> Result<Vec<String>, GrantsError<Reads>> {
        let fresh = auth.mcp.requires_fresh_policy;
        let server_ids = self
            .reads
            .access_group_server_ids(&auth.access_group_ids, fresh)
            .await?;
        if server_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .reads
            .registry()
            .expand_permission_list(&server_ids)
            .await)
    }
}
