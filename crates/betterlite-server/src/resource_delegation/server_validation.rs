use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error as StdError;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamFields;
use betterlite_mcp::access_group_servers::AccessGroupServerRows;
use betterlite_mcp::server_record::MCPServer;
use thiserror::Error;
use tracing::warn;

use crate::mcp_identifier_resolution::{ResolveError, resolve_identifiers_to_ids};
use crate::mcp_permission_ids::cleanup::GrantCleanupError;
use crate::mcp_permission_ids::cleanup::drop_stale_object_permission_mcp_identifiers as drop_stale_identifiers; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses
use crate::mcp_permission_ids::cleanup::flatten_resolved_mcp_server_ids as flatten_resolved_ids; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses
use crate::mcp_permission_ids::registry::allow_all_keys_server_ids;

use super::GrantReads;
use super::ObjectPermissionGrants;
use super::denial::Denial;
use super::gates::validate_requested_toolsets;
use super::grandfather::GrandfatherError;
use super::grandfather::grandfathered_server_ids;
use super::quoted_list;
use super::requested::RequestedError;
use super::requested::requested_entries;
use super::requested::requested_server_ids;
use super::team_read::TeamReadError;
use super::team_read::team_allowed_server_ids;

#[derive(Debug, Error)]
pub enum KeyServersError<RowError>
where
    RowError: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Requested(#[from] RequestedError),
    #[error(transparent)]
    TeamRead(#[from] TeamReadError<RowError>),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Cleanup(#[from] GrantCleanupError),
    #[error(transparent)]
    Grandfather(#[from] GrandfatherError),
    #[error(transparent)]
    Denied(#[from] Denial),
}

struct ServerScope<'grants> {
    team: Option<&'grants TeamFields>,
    allow_all: &'grants BTreeSet<String>,
    team_allowed: &'grants BTreeSet<String>,
    existing: Option<&'grants ObjectPermissionGrants>,
    teamless_admin: bool,
}

fn team_denial(
    team_id: &str,
    resource: &str,
    disallowed: &BTreeSet<String>,
    team_allowed: &BTreeSet<String>,
    globals: Option<&BTreeSet<String>>,
) -> Denial {
    let prefix = format!(
        "Key requests {resource} not allowed by team '{team_id}': {}. Team allows: {}.",
        quoted_list(disallowed),
        quoted_list(team_allowed)
    );
    let globals_text = globals.map_or_else(String::new, |global_ids| {
        format!(
            " Global (allow_all_keys) servers: {}.",
            quoted_list(global_ids)
        )
    });
    Denial {
        detail: format!("{prefix}{globals_text}"),
    }
}

fn permitted_grants(
    ceiling: &BTreeSet<String>,
    requested: &BTreeSet<String>,
    teamless_admin: bool,
) -> BTreeSet<String> {
    let mut permitted = ceiling.clone();
    if teamless_admin {
        permitted.extend(requested.iter().cloned());
    }
    permitted
}

fn check_requested_groups(
    team: Option<&TeamFields>,
    requested: &BTreeSet<String>,
    teamless_admin: bool,
) -> Result<(), Denial> {
    let team_groups: BTreeSet<String> = team
        .and_then(|fields| fields.object_permission.as_ref())
        .and_then(|permission| permission.mcp_access_groups.0.as_deref())
        .map(|groups| groups.iter().cloned().collect())
        .unwrap_or_default();
    let permitted = permitted_grants(&team_groups, requested, teamless_admin);
    let disallowed: BTreeSet<String> = requested.difference(&permitted).cloned().collect();
    if disallowed.is_empty() {
        return Ok(());
    }
    let Some(fields) = team else {
        return Err(Denial {
            detail: format!(
                "Key is not in a team. MCP access groups cannot be assigned to keys outside of a team. Disallowed groups: {}.",
                quoted_list(&disallowed)
            ),
        });
    };
    Err(team_denial(
        &fields.base.team_id,
        "MCP access groups",
        &disallowed,
        &team_groups,
        None,
    ))
}

fn drop_stale_servers(
    permission: &mut JsonData,
    requested: &BTreeSet<String>,
    resolved: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), GrantCleanupError> {
    let stale: Vec<String> = requested
        .iter()
        .filter(|identifier| resolved.get(*identifier).is_none_or(BTreeSet::is_empty))
        .cloned()
        .collect();
    if !stale.is_empty() {
        let stale_text = quoted_list(&stale);
        warn!(
            "validate_key_mcp_servers_against_team: ignoring stale MCP server identifiers (no longer in registry or DB): {stale_text}"
        );
    }
    drop_stale_identifiers(permission, resolved)
}

async fn check_requested_servers<Rows>(
    permission: &mut JsonData,
    requested: &BTreeSet<String>,
    scope: &ServerScope<'_>,
    reads: &GrantReads<'_, Rows>,
) -> Result<(), KeyServersError<Rows::Error>>
where
    Rows: AccessGroupServerRows,
{
    let registry_servers: Vec<MCPServer> = {
        let guard = reads.registry.read().await;
        guard.get_registry().into_iter().cloned().collect()
    };
    let refs: Vec<&MCPServer> = registry_servers.iter().collect();
    let resolved = resolve_identifiers_to_ids(requested, reads.client, &refs).await?;
    drop_stale_servers(permission, requested, &resolved)?;
    let active = flatten_resolved_ids(&resolved);
    let mut ceiling = scope.team_allowed.clone();
    ceiling.extend(scope.allow_all.iter().cloned());
    let permitted = permitted_grants(&ceiling, &active, scope.teamless_admin);
    let grandfathered =
        grandfathered_server_ids(scope.existing, reads.client, reads.registry).await?;
    let disallowed: BTreeSet<String> = active
        .iter()
        .filter(|server_id| !permitted.contains(*server_id) && !grandfathered.contains(*server_id))
        .cloned()
        .collect();
    if disallowed.is_empty() {
        return Ok(());
    }
    let Some(team) = scope.team else {
        return Err(KeyServersError::Denied(Denial {
            detail: format!(
                "Key is not in a team. Only globally available (allow_all_keys) MCP servers can be assigned: {}. Disallowed servers: {}.",
                quoted_list(scope.allow_all),
                quoted_list(&disallowed)
            ),
        }));
    };
    Err(KeyServersError::Denied(team_denial(
        &team.base.team_id,
        "MCP servers",
        &disallowed,
        scope.team_allowed,
        Some(scope.allow_all),
    )))
}

pub async fn validate_key_servers<Rows>(
    permission: Option<&mut JsonData>,
    team: Option<&TeamFields>,
    reads: GrantReads<'_, Rows>,
    is_proxy_admin: bool,
    existing_key_permission: Option<&ObjectPermissionGrants>,
) -> Result<(), KeyServersError<Rows::Error>>
where
    Rows: AccessGroupServerRows,
{
    let server_ids = requested_server_ids(permission.as_deref())?;
    let access_groups: BTreeSet<String> =
        requested_entries("mcp_access_groups", permission.as_deref(), false)?;
    let toolsets: BTreeSet<String> =
        requested_entries("mcp_toolsets", permission.as_deref(), false)?;
    if server_ids.is_empty() && access_groups.is_empty() && toolsets.is_empty() {
        return Ok(());
    }
    let Some(permission_value) = permission else {
        return Ok(());
    };
    let allow_all = {
        let guard = reads.registry.read().await;
        allow_all_keys_server_ids(&guard)
    };
    let team_allowed = team_allowed_server_ids(team, &reads).await?;
    let scope = ServerScope {
        team,
        allow_all: &allow_all,
        team_allowed: &team_allowed,
        existing: existing_key_permission,
        teamless_admin: team.is_none() && is_proxy_admin,
    };
    if !server_ids.is_empty() {
        check_requested_servers(permission_value, &server_ids, &scope, &reads).await?;
    }
    if !access_groups.is_empty() {
        check_requested_groups(team, &access_groups, scope.teamless_admin)?;
    }
    validate_requested_toolsets(&toolsets, team, is_proxy_admin).map_err(KeyServersError::Denied)
}
