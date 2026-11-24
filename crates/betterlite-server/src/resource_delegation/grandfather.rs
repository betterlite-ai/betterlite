use std::collections::BTreeSet;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::registry::servers::SharedRegistry;
use betterlite_mcp::server_record::MCPServer;
use thiserror::Error;

use crate::mcp_identifier_resolution::{ResolveError, resolve_identifiers_to_ids};
use crate::mcp_permission_ids::cleanup::NO_MCP_SERVERS;
use crate::mcp_permission_ids::cleanup::flatten_resolved_mcp_server_ids as flatten_resolved_ids; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses
use crate::mcp_permission_ids::decode::{McpToolGrantsJsonError, decode_mcp_tool_grants_json};
use crate::mcp_permission_ids::team::ALL_PROXY_MCP_SERVERS;

use super::ObjectPermissionGrants;
use super::registry_servers;

#[derive(Debug, Error)]
pub enum GrandfatherError {
    #[error(transparent)]
    ToolGrants(#[from] McpToolGrantsJsonError),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
}

pub async fn grandfathered_server_ids(
    existing: Option<&ObjectPermissionGrants>,
    client: Option<PrismaClient<'_>>,
    registry: &SharedRegistry,
) -> Result<BTreeSet<String>, GrandfatherError> {
    let Some(existing) = existing else {
        return Ok(BTreeSet::new());
    };
    if client.is_none() {
        return Ok(BTreeSet::new());
    }
    let mut identifiers: BTreeSet<String> =
        existing.mcp_servers.iter().flatten().cloned().collect();
    // Python's `or {}` skips the stored-JSON decoder for a falsey column before it reads any keys
    if let Some(raw) = &existing.mcp_tool_permissions
        && !is_falsey(raw)
    {
        identifiers.extend(stored_tool_keys(raw)?);
    }
    identifiers.remove(NO_MCP_SERVERS);
    identifiers.remove(ALL_PROXY_MCP_SERVERS);
    let servers = registry_servers(registry).await;
    let refs: Vec<&MCPServer> = servers.iter().collect();
    let resolved = resolve_identifiers_to_ids(&identifiers, client, &refs).await?;
    Ok(flatten_resolved_ids(&resolved))
}

fn is_falsey(value: &JsonData) -> bool {
    match value {
        JsonData::Null => true,
        JsonData::Bool(flag) => !*flag,
        JsonData::Number(number) => number.as_f64() == Some(0.0),
        JsonData::String(text) => text.is_empty(),
        JsonData::Array(items) => items.is_empty(),
        JsonData::Object(fields) => fields.is_empty(),
    }
}

fn stored_tool_keys(raw: &JsonData) -> Result<BTreeSet<String>, McpToolGrantsJsonError> {
    match raw {
        JsonData::String(text) => Ok(decode_mcp_tool_grants_json(text.as_bytes())?
            .into_keys()
            .collect()),
        JsonData::Object(fields) => Ok(fields.keys().cloned().collect()),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => {
            Err(McpToolGrantsJsonError::NotAnObject)
        }
    }
}
