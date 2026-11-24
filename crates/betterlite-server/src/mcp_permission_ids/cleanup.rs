use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use thiserror::Error;

pub const NO_MCP_SERVERS: &str = "no-mcp-servers";

#[derive(Debug, Error)]
pub enum GrantCleanupError {
    #[error("mcp_servers entry {index} is not a server identifier string")]
    ServerIdentifier { index: usize },
    #[error("mcp_tool_permissions entry '{identifier}' holds a tool name that is not a string")]
    ToolName { identifier: String },
}

#[must_use]
pub fn dedupe_preserving_order(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            result.push(value.clone());
        }
    }
    result
}

// ast-grep-ignore: no-long-names - the LiteLLM function this task owns
pub fn drop_stale_object_permission_mcp_servers(
    object_permission: &mut JsonData,
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), GrantCleanupError> {
    let JsonData::Object(fields) = object_permission else {
        return Ok(());
    };
    let Some(JsonData::Array(servers)) = fields.get("mcp_servers") else {
        return Ok(());
    };
    // The stored grant keeps the caller's identifiers, since a shared database expands aliases per region
    let mut kept = Vec::new();
    for (index, server) in servers.iter().enumerate() {
        let JsonData::String(identifier) = server else {
            return Err(GrantCleanupError::ServerIdentifier { index });
        };
        let resolves = identifier_to_server_ids
            .get(identifier)
            .is_some_and(|server_ids| !server_ids.is_empty());
        if identifier == NO_MCP_SERVERS || resolves {
            kept.push(identifier.clone());
        }
    }
    let deduped = dedupe_preserving_order(&kept);
    let deduped_servers = JsonData::Array(deduped.into_iter().map(JsonData::String).collect());
    fields.insert("mcp_servers".to_owned(), deduped_servers);
    Ok(())
}

// ast-grep-ignore: no-long-names - the LiteLLM function this task owns
pub fn drop_stale_object_permission_mcp_tool_permissions(
    object_permission: &mut JsonData,
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), GrantCleanupError> {
    let JsonData::Object(fields) = object_permission else {
        return Ok(());
    };
    let Some(JsonData::Object(granted)) = fields.get("mcp_tool_permissions") else {
        return Ok(());
    };
    let mut kept = BTreeMap::new();
    for (identifier, tools) in granted {
        let resolves = identifier_to_server_ids
            .get(identifier)
            .is_some_and(|server_ids| !server_ids.is_empty());
        if !resolves {
            continue;
        }
        let names = match tools {
            JsonData::Array(items) => deduped_tool_names(items, identifier)?,
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Object(_) => Vec::new(),
        };
        kept.insert(
            identifier.clone(),
            JsonData::Array(names.into_iter().map(JsonData::String).collect()),
        );
    }
    let grants = JsonData::Object(kept);
    fields.insert("mcp_tool_permissions".to_owned(), grants);
    Ok(())
}

// ast-grep-ignore: no-long-names - the LiteLLM function this task owns
pub fn drop_stale_object_permission_mcp_identifiers(
    object_permission: &mut JsonData,
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), GrantCleanupError> {
    if !matches!(object_permission, JsonData::Object(fields) if !fields.is_empty()) {
        return Ok(());
    }
    drop_stale_object_permission_mcp_servers(object_permission, identifier_to_server_ids)?; // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
    drop_stale_object_permission_mcp_tool_permissions(object_permission, identifier_to_server_ids) // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
}

#[must_use]
// ast-grep-ignore: no-long-names - the LiteLLM function this task owns
pub fn flatten_resolved_mcp_server_ids(
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut server_ids = BTreeSet::new();
    for ids in identifier_to_server_ids.values() {
        server_ids.extend(ids.iter().cloned());
    }
    server_ids
}

fn deduped_tool_names(
    items: &[JsonData],
    identifier: &str,
) -> Result<Vec<String>, GrantCleanupError> {
    let mut names = Vec::new();
    for item in items {
        let JsonData::String(name) = item else {
            return Err(GrantCleanupError::ToolName {
                identifier: identifier.to_owned(),
            });
        };
        names.push(name.clone());
    }
    Ok(dedupe_preserving_order(&names))
}
