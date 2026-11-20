use std::collections::BTreeMap;
use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::server_record::MCPServer;
use thiserror::Error;

use crate::mcp_identifier_resolution::ResolveError;
use crate::mcp_identifier_resolution::resolve_identifiers_to_ids;

use super::entries::ToolPermissionEntries;
use super::entries::ToolPermissionEntriesError;
use super::entries::mcp_tool_permission_entries;

#[derive(Debug, Error)]
pub enum AmbiguityError {
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Entries(#[from] ToolPermissionEntriesError),
    #[error(
        "Ambiguous mcp_tool_permissions key: {collisions}. Key tool permissions by server_id when servers share a name or alias."
    )]
    Ambiguous { collisions: String },
}

impl AmbiguityError {
    #[must_use]
    pub const fn status_code(&self) -> Option<StatusCode> {
        match self {
            Self::Ambiguous { .. } => Some(StatusCode::BAD_REQUEST),
            Self::Resolve(_) | Self::Entries(_) => None,
        }
    }
}

// ast-grep-ignore: no-long-names
pub async fn reject_ambiguous_mcp_tool_permission_keys(
    new_mcp_tool_permissions: Option<&JsonData>,
    existing_mcp_tool_permissions: Option<&JsonData>,
    client: Option<PrismaClient<'_>>,
    registry: &[&MCPServer],
) -> Result<(), AmbiguityError> {
    let requested = mcp_tool_permission_entries(new_mcp_tool_permissions)?;
    let stored = mcp_tool_permission_entries(existing_mcp_tool_permissions)?;
    let changed = changed_identifiers(&requested, &stored);
    let resolved = resolve_identifiers_to_ids(&changed, client, registry).await?;
    let collisions = render_collisions(&resolved);
    if collisions.is_empty() {
        return Ok(());
    }
    Err(AmbiguityError::Ambiguous { collisions })
}

fn changed_identifiers(
    requested: &ToolPermissionEntries,
    stored: &ToolPermissionEntries,
) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    for (identifier, tools) in requested {
        if stored.get(identifier) != Some(tools) {
            changed.insert(identifier.clone());
        }
    }
    changed
}

fn render_collisions(resolved: &BTreeMap<String, BTreeSet<String>>) -> String {
    let mut collisions = Vec::new();
    for (identifier, server_ids) in resolved {
        if server_ids.len() > 1 && !server_ids.contains(identifier) {
            collisions.push(format!(
                "'{identifier}' matches MCP servers {}",
                quoted_server_ids(server_ids)
            ));
        }
    }
    collisions.join("; ")
}

fn quoted_server_ids(server_ids: &BTreeSet<String>) -> String {
    let quoted: Vec<String> = server_ids
        .iter()
        .map(|server_id| format!("'{server_id}'"))
        .collect();
    format!("[{}]", quoted.join(", "))
}
