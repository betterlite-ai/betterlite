use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use thiserror::Error;

use crate::mcp_permission_ids::cleanup::NO_MCP_SERVERS;

#[derive(Debug, Error)]
pub enum RequestedError {
    #[error("{field} entry {index} is not a string")]
    NotAString { field: &'static str, index: usize },
}

fn string_list(
    field: &'static str,
    permission: Option<&JsonData>,
) -> Result<Vec<String>, RequestedError> {
    let Some(JsonData::Object(fields)) = permission else {
        return Ok(Vec::new());
    };
    let Some(JsonData::Array(items)) = fields.get(field) else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| match item {
            JsonData::String(value) => Ok(value.clone()),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_) => Err(RequestedError::NotAString { field, index }),
        })
        .collect()
}

// LiteLLM keeps empty entries for access groups and toolsets, and skips them only for vector stores and search tools.
pub fn requested_entries<Collection>(
    field: &'static str,
    permission: Option<&JsonData>,
    drop_empty: bool,
) -> Result<Collection, RequestedError>
where
    Collection: FromIterator<String>,
{
    let listed = string_list(field, permission)?;
    if !drop_empty {
        return Ok(listed.into_iter().collect());
    }
    Ok(listed
        .into_iter()
        .filter(|entry| !entry.is_empty())
        .collect())
}

pub fn requested_server_ids(
    permission: Option<&JsonData>,
) -> Result<BTreeSet<String>, RequestedError> {
    let mut server_ids: BTreeSet<String> = requested_entries("mcp_servers", permission, false)?;
    server_ids.remove(NO_MCP_SERVERS);
    if let Some(JsonData::Object(fields)) = permission
        && let Some(JsonData::Object(grants)) = fields.get("mcp_tool_permissions")
    {
        server_ids.extend(grants.keys().cloned());
    }
    Ok(server_ids)
}
