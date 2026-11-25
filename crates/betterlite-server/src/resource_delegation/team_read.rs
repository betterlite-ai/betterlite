use std::collections::BTreeSet;
use std::error::Error as StdError;

use betterlite_auth::auth_model_patterns::access_groups::{ResourceField, collect_resources};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamFields;
use betterlite_mcp::access_group_servers::AccessGroupServerRows;
use betterlite_mcp::server_record::MCPServer;
use thiserror::Error;

use crate::mcp_identifier_resolution::{ResolveError, resolve_identifiers_to_ids};
use crate::mcp_permission_ids::cleanup::flatten_resolved_mcp_server_ids as flatten_resolved_ids; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses
use crate::mcp_permission_ids::team::TeamAllowedServersError;
use crate::mcp_permission_ids::team::TeamObjectPermission;
use crate::mcp_permission_ids::team::resolve_team_allowed_mcp_servers as resolve_direct_team_grants; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses

use super::GrantReads;

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

async fn unified_server_ids<Rows>(
    access_group_ids: &[String],
    reads: &GrantReads<'_, Rows>,
) -> Result<BTreeSet<String>, TeamReadError<Rows::Error>>
where
    Rows: AccessGroupServerRows,
{
    // B747's lookup keeps an unavailable group out of the result unless the caller reads the database only.
    let unified_ids = collect_resources(
        access_group_ids,
        ResourceField::McpServerIds,
        reads.unified,
        false,
    )
    .unwrap_or_default();
    let identifiers: BTreeSet<String> = unified_ids.iter().cloned().collect();
    let registry_servers: Vec<MCPServer> = {
        let guard = reads.registry.read().await;
        guard.get_registry().into_iter().cloned().collect()
    };
    let refs: Vec<&MCPServer> = registry_servers.iter().collect();
    let resolved = resolve_identifiers_to_ids(&identifiers, reads.client, &refs).await?;
    let mut allowed = flatten_resolved_ids(&resolved);
    for identifier in unified_ids {
        if resolved.get(&identifier).is_none_or(BTreeSet::is_empty) {
            allowed.insert(identifier);
        }
    }
    Ok(allowed)
}

pub async fn team_allowed_server_ids<Rows>(
    team: Option<&TeamFields>,
    reads: &GrantReads<'_, Rows>,
) -> Result<BTreeSet<String>, TeamReadError<Rows::Error>>
where
    Rows: AccessGroupServerRows,
{
    let Some(fields) = team else {
        return Ok(BTreeSet::new());
    };
    let access_group_ids = fields.base.access_group_ids.as_deref().unwrap_or_default();
    let mut allowed = unified_server_ids(access_group_ids, reads).await?;
    let Some(permission) = fields.object_permission.as_ref() else {
        return Ok(allowed);
    };
    let direct = resolve_direct_team_grants(
        &TeamObjectPermission {
            mcp_servers: permission.mcp_servers.0.clone(),
            mcp_access_groups: permission.mcp_access_groups.0.clone(),
            mcp_tool_permissions: permission.mcp_tool_permissions.as_ref().map(|grants| {
                JsonData::Object(
                    grants
                        .iter()
                        .map(|(server, tools)| {
                            (
                                server.clone(),
                                JsonData::Array(
                                    tools.iter().cloned().map(JsonData::String).collect(),
                                ),
                            )
                        })
                        .collect(),
                )
            }),
        },
        reads.client,
        reads.registry,
        reads.rows,
    )
    .await?;
    allowed.extend(direct);
    Ok(allowed)
}
