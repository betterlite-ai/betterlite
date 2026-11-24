use std::collections::BTreeSet;
use std::error::Error as StdError;

use betterlite_auth::auth_model_patterns::access_groups::{ResourceField, collect_resources};
use betterlite_mcp::access_group_servers::AccessGroupServerRows;
use betterlite_mcp::server_record::MCPServer;
use thiserror::Error;

use crate::mcp_identifier_resolution::{ResolveError, resolve_identifiers_to_ids};
use crate::mcp_permission_ids::cleanup::flatten_resolved_mcp_server_ids as flatten_resolved_ids; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses
use crate::mcp_permission_ids::team::TeamAllowedServersError;
use crate::mcp_permission_ids::team::TeamObjectPermission;
use crate::mcp_permission_ids::team::resolve_team_allowed_mcp_servers as resolve_direct_team_grants; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses

use super::GrantReads;
use super::ObjectPermissionGrants;
use super::TeamGrants;
use super::registry_servers;

#[derive(Debug, Error)]
pub enum TeamReadError<RowError>
where
    RowError: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Direct(#[from] TeamAllowedServersError<RowError>),
}

fn direct_permission(permission: &ObjectPermissionGrants) -> TeamObjectPermission {
    TeamObjectPermission {
        mcp_servers: permission.mcp_servers.clone(),
        mcp_access_groups: permission.mcp_access_groups.clone(),
        mcp_tool_permissions: permission.mcp_tool_permissions.clone(),
    }
}

pub async fn team_allowed_server_ids<Rows>(
    team: Option<&TeamGrants>,
    reads: &GrantReads<'_, Rows>,
) -> Result<BTreeSet<String>, TeamReadError<Rows::Error>>
where
    Rows: AccessGroupServerRows,
{
    let Some(team) = team else {
        return Ok(BTreeSet::new());
    };
    // B747's lookup keeps an unavailable group out of the result unless the caller reads the database only.
    let unified_ids = collect_resources(
        &team.access_group_ids,
        ResourceField::McpServerIds,
        reads.unified,
        false,
    )
    .unwrap_or_default();
    let identifiers: BTreeSet<String> = unified_ids.iter().cloned().collect();
    let servers = registry_servers(reads.registry).await;
    let refs: Vec<&MCPServer> = servers.iter().collect();
    let resolved = resolve_identifiers_to_ids(&identifiers, reads.client, &refs).await?;
    let mut allowed = flatten_resolved_ids(&resolved);
    for identifier in unified_ids {
        if resolved.get(&identifier).is_none_or(BTreeSet::is_empty) {
            allowed.insert(identifier);
        }
    }
    let Some(permission) = team.object_permission.as_ref() else {
        return Ok(allowed);
    };
    let direct = resolve_direct_team_grants(
        &direct_permission(permission),
        reads.client,
        reads.registry,
        reads.rows,
    )
    .await?;
    allowed.extend(direct);
    Ok(allowed)
}
