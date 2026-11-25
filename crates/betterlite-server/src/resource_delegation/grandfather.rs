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
    let Some(stored) = existing else {
        return Ok(BTreeSet::new());
    };
    if client.is_none() {
        return Ok(BTreeSet::new());
    }
    let mut identifiers: BTreeSet<String> = stored.mcp_servers.iter().cloned().collect();
    identifiers.extend(stored_tool_keys(&stored.mcp_tool_permissions)?);
    identifiers.remove(NO_MCP_SERVERS);
    identifiers.remove(ALL_PROXY_MCP_SERVERS);
    let registry_servers: Vec<MCPServer> = {
        let guard = registry.read().await;
        guard.get_registry().into_iter().cloned().collect()
    };
    let refs: Vec<&MCPServer> = registry_servers.iter().collect();
    let resolved = resolve_identifiers_to_ids(&identifiers, client, &refs).await?;
    Ok(flatten_resolved_ids(&resolved))
}

fn stored_tool_keys(raw: &JsonData) -> Result<BTreeSet<String>, McpToolGrantsJsonError> {
    match raw {
        JsonData::Null => Ok(BTreeSet::new()),
        JsonData::Object(fields) => Ok(fields.keys().cloned().collect()),
        JsonData::String(text) => Ok(decode_mcp_tool_grants_json(text.as_bytes())?
            .into_keys()
            .collect()),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => {
            Err(McpToolGrantsJsonError::NotAnObject)
        }
    }
}
