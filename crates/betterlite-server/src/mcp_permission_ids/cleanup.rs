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

#[derive(Clone, Copy)]
enum InvalidItem<'identifier> {
    ServerIdentifier,
    ToolName { identifier: &'identifier str },
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

fn resolving(
    identifier: &str,
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    identifier_to_server_ids
        .get(identifier)
        .is_some_and(|server_ids| !server_ids.is_empty())
}

fn string_values(
    items: &[JsonData],
    invalid_item: InvalidItem<'_>,
) -> Result<Vec<String>, GrantCleanupError> {
    let mut values = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let JsonData::String(value) = item else {
            return Err(match invalid_item {
                InvalidItem::ServerIdentifier => GrantCleanupError::ServerIdentifier { index },
                InvalidItem::ToolName { identifier } => GrantCleanupError::ToolName {
                    identifier: identifier.to_owned(),
                },
            });
        };
        values.push(value.clone());
    }
    Ok(values)
}

fn kept_server_identifiers(
    servers: &[JsonData],
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<String>, GrantCleanupError> {
    let identifiers = string_values(servers, InvalidItem::ServerIdentifier)?;
    Ok(identifiers
        .into_iter()
        .filter(|identifier| {
            identifier.as_str() == NO_MCP_SERVERS || resolving(identifier, identifier_to_server_ids)
        })
        .collect())
}

fn kept_tool_grants(
    granted: &BTreeMap<String, JsonData>,
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeMap<String, JsonData>, GrantCleanupError> {
    let mut kept = BTreeMap::new();
    for (identifier, tools) in granted {
        if !resolving(identifier, identifier_to_server_ids) {
            continue;
        }
        let names = match tools {
            JsonData::Array(items) => {
                let names = string_values(
                    items,
                    InvalidItem::ToolName {
                        identifier: identifier.as_str(),
                    },
                )?;
                dedupe_preserving_order(&names)
            }
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
    Ok(kept)
}

// ast-grep-ignore: no-long-names - the LiteLLM function this task owns
pub fn drop_stale_object_permission_mcp_identifiers(
    object_permission: &mut JsonData,
    identifier_to_server_ids: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), GrantCleanupError> {
    let JsonData::Object(fields) = object_permission else {
        return Ok(());
    };
    if fields.is_empty() {
        return Ok(());
    }
    if let Some(JsonData::Array(servers)) = fields.get("mcp_servers") {
        let kept = kept_server_identifiers(servers, identifier_to_server_ids)?;
        let deduped = dedupe_preserving_order(&kept);
        let cleaned = JsonData::Array(deduped.into_iter().map(JsonData::String).collect());
        fields.insert("mcp_servers".to_owned(), cleaned);
    }
    if let Some(JsonData::Object(granted)) = fields.get("mcp_tool_permissions") {
        let cleaned = JsonData::Object(kept_tool_grants(granted, identifier_to_server_ids)?);
        fields.insert("mcp_tool_permissions".to_owned(), cleaned);
    }
    Ok(())
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
