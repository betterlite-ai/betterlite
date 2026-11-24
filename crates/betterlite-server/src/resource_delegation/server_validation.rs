use std::collections::BTreeSet;
use std::error::Error as StdError;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::access_group_servers::AccessGroupServerRows;
use betterlite_mcp::registry::servers::SharedRegistry;
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
use super::TeamGrants;
use super::denial::Denial;
use super::gates::validate_requested_toolsets;
use super::grandfather::GrandfatherError;
use super::grandfather::grandfathered_server_ids;
use super::quoted_list;
use super::registry_servers;
use super::requested::RequestedError;
use super::requested::requested_access_groups;
use super::requested::requested_server_ids;
use super::requested::requested_toolsets;
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

struct ServerScope<'a> {
    team: Option<&'a TeamGrants>,
    allow_all: &'a BTreeSet<String>,
    team_allowed: &'a BTreeSet<String>,
    existing: Option<&'a ObjectPermissionGrants>,
    teamless_admin: bool,
}

fn server_denial(
    team: Option<&TeamGrants>,
    disallowed: &BTreeSet<String>,
    team_allowed: &BTreeSet<String>,
    allow_all: &BTreeSet<String>,
) -> Denial {
    let disallowed_text = quoted_list(disallowed);
    match team {
        Some(team) => {
            let team_id = team.team_id.as_str();
            let team_text = quoted_list(team_allowed);
            let global_text = quoted_list(allow_all);
            Denial {
                detail: format!(
                    "Key requests MCP servers not allowed by team '{team_id}': {disallowed_text}. Team allows: {team_text}. Global (allow_all_keys) servers: {global_text}."
                ),
            }
        }
        None => {
            let global_text = quoted_list(allow_all);
            Denial {
                detail: format!(
                    "Key is not in a team. Only globally available (allow_all_keys) MCP servers can be assigned: {global_text}. Disallowed servers: {disallowed_text}."
                ),
            }
        }
    }
}

fn group_denial(
    team: Option<&TeamGrants>,
    disallowed: &BTreeSet<String>,
    team_groups: &BTreeSet<String>,
) -> Denial {
    let disallowed_text = quoted_list(disallowed);
    match team {
        Some(team) => {
            let team_id = team.team_id.as_str();
            let team_text = quoted_list(team_groups);
            Denial {
                detail: format!(
                    "Key requests MCP access groups not allowed by team '{team_id}': {disallowed_text}. Team allows: {team_text}."
                ),
            }
        }
        None => Denial {
            detail: format!(
                "Key is not in a team. MCP access groups cannot be assigned to keys outside of a team. Disallowed groups: {disallowed_text}."
            ),
        },
    }
}

fn check_requested_groups(
    team: Option<&TeamGrants>,
    requested: &BTreeSet<String>,
    teamless_admin: bool,
) -> Result<(), Denial> {
    let team_groups: BTreeSet<String> = team
        .and_then(|team| team.object_permission.as_ref())
        .and_then(|permission| permission.mcp_access_groups.as_ref())
        .map(|groups| groups.iter().cloned().collect())
        .unwrap_or_default();
    let mut permitted = team_groups.clone();
    if teamless_admin {
        permitted.extend(requested.iter().cloned());
    }
    let disallowed: BTreeSet<String> = requested.difference(&permitted).cloned().collect();
    if disallowed.is_empty() {
        return Ok(());
    }
    Err(group_denial(team, &disallowed, &team_groups))
}

async fn allow_all_keys_from(registry: &SharedRegistry) -> BTreeSet<String> {
    let guard = registry.read().await;
    allow_all_keys_server_ids(&guard)
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
    let servers = registry_servers(reads.registry).await;
    let refs: Vec<&MCPServer> = servers.iter().collect();
    let resolved = resolve_identifiers_to_ids(requested, reads.client, &refs).await?;
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
    drop_stale_identifiers(permission, &resolved)?;
    let active = flatten_resolved_ids(&resolved);
    let mut permitted: BTreeSet<String> = scope.team_allowed.clone();
    permitted.extend(scope.allow_all.iter().cloned());
    if scope.teamless_admin {
        permitted.extend(active.iter().cloned());
    }
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
    Err(KeyServersError::Denied(server_denial(
        scope.team,
        &disallowed,
        scope.team_allowed,
        scope.allow_all,
    )))
}

pub async fn validate_key_servers<Rows>(
    permission: Option<&mut JsonData>,
    team: Option<&TeamGrants>,
    reads: GrantReads<'_, Rows>,
    is_proxy_admin: bool,
    existing_key_permission: Option<&ObjectPermissionGrants>,
) -> Result<(), KeyServersError<Rows::Error>>
where
    Rows: AccessGroupServerRows,
{
    let server_ids = requested_server_ids(permission.as_deref())?;
    let access_groups = requested_access_groups(permission.as_deref())?;
    let toolsets = requested_toolsets(permission.as_deref())?;
    if server_ids.is_empty() && access_groups.is_empty() && toolsets.is_empty() {
        return Ok(());
    }
    let Some(permission) = permission else {
        return Ok(());
    };
    let allow_all = allow_all_keys_from(reads.registry).await;
    let team_allowed = team_allowed_server_ids(team, &reads).await?;
    let scope = ServerScope {
        team,
        allow_all: &allow_all,
        team_allowed: &team_allowed,
        existing: existing_key_permission,
        teamless_admin: team.is_none() && is_proxy_admin,
    };
    if !server_ids.is_empty() {
        check_requested_servers(permission, &server_ids, &scope, &reads).await?;
    }
    if !access_groups.is_empty() {
        check_requested_groups(team, &access_groups, scope.teamless_admin)?;
    }
    validate_requested_toolsets(&toolsets, team, is_proxy_admin).map_err(KeyServersError::Denied)
}
