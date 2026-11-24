use std::collections::BTreeSet;
use std::error::Error as StdError;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::access_group_servers::{
    AccessGroupServerRows, AccessGroupServersError, servers_from_access_groups,
};
use betterlite_mcp::registry::servers::SharedRegistry;
use betterlite_mcp::server_record::MCPServer;
use thiserror::Error;

use crate::mcp_identifier_resolution::{ResolveError, resolve_identifiers_to_ids};

use super::cleanup::flatten_resolved_mcp_server_ids; // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
use super::decode::{McpToolGrantsJsonError, decode_mcp_tool_grants_json};

pub const ALL_PROXY_MCP_SERVERS: &str = "all-proxy-mcpservers";

#[derive(Clone, Debug, Default)]
pub struct TeamObjectPermission {
    pub mcp_servers: Option<Vec<String>>,
    pub mcp_access_groups: Option<Vec<String>>,
    pub mcp_tool_permissions: Option<JsonData>,
}

#[derive(Debug, Error)]
pub enum TeamAllowedServersError<RowError>
where
    RowError: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    AccessGroups(#[from] AccessGroupServersError<RowError>),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    ToolGrants(#[from] McpToolGrantsJsonError),
}

// ast-grep-ignore: no-long-names - the LiteLLM function this task owns
pub async fn resolve_team_allowed_mcp_servers<R>(
    team_object_permission: &TeamObjectPermission,
    client: Option<PrismaClient<'_>>,
    registry: &SharedRegistry,
    rows: Option<&R>,
) -> Result<BTreeSet<String>, TeamAllowedServersError<R::Error>>
where
    R: AccessGroupServerRows,
{
    let direct_servers = team_object_permission
        .mcp_servers
        .as_deref()
        .unwrap_or_default();
    if direct_servers
        .iter()
        .any(|server| server == ALL_PROXY_MCP_SERVERS)
    {
        return Ok(registry.read().await.get_all_mcp_server_ids());
    }
    let access_groups = team_object_permission
        .mcp_access_groups
        .as_deref()
        .unwrap_or_default();
    let access_group_servers =
        servers_from_access_groups(registry, rows, access_groups, false).await?;
    let tool_identifiers =
        tool_grant_identifiers(team_object_permission.mcp_tool_permissions.as_ref())?;
    let mut identifiers = BTreeSet::new();
    identifiers.extend(direct_servers.iter().cloned());
    identifiers.extend(access_group_servers);
    identifiers.extend(tool_identifiers);
    // LiteLLM defaults prisma_client to null, so a caller without a database resolves from the registry alone
    let registry_servers: Vec<MCPServer> = {
        let guard = registry.read().await;
        guard.get_registry().into_iter().cloned().collect()
    };
    let registry_refs: Vec<&MCPServer> = registry_servers.iter().collect();
    let resolved = resolve_identifiers_to_ids(&identifiers, client, &registry_refs).await?;
    let mut allowed = flatten_resolved_mcp_server_ids(&resolved); // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
    for identifier in identifiers {
        if resolved.get(&identifier).is_none_or(BTreeSet::is_empty) {
            allowed.insert(identifier);
        }
    }
    Ok(allowed)
}

fn tool_grant_identifiers(
    raw: Option<&JsonData>,
) -> Result<BTreeSet<String>, McpToolGrantsJsonError> {
    let Some(value) = raw else {
        return Ok(BTreeSet::new());
    };
    if is_falsey(value) {
        return Ok(BTreeSet::new());
    }
    match value {
        JsonData::String(text) => {
            let grants = decode_mcp_tool_grants_json(text.as_bytes())?;
            Ok(grants.into_keys().collect())
        }
        JsonData::Object(fields) => Ok(fields.keys().cloned().collect()),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => {
            Err(McpToolGrantsJsonError::NotAnObject)
        }
    }
}

// Python's `or {}` on the raw column, which is why only a falsey value skips the stored-JSON decoder
fn is_falsey(value: &JsonData) -> bool {
    match value {
        JsonData::Null => true,
        JsonData::Bool(flag) => !flag,
        JsonData::Number(number) => {
            let zero_bits = [0_u64, 0x8000_0000_0000_0000];
            number.as_i64() == Some(0)
                || number
                    .as_f64()
                    .is_some_and(|float| zero_bits.contains(&float.to_bits()))
        }
        JsonData::String(text) => text.is_empty(),
        JsonData::Array(items) => items.is_empty(),
        JsonData::Object(fields) => fields.is_empty(),
    }
}
