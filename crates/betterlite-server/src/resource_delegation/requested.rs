use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use thiserror::Error;

use crate::mcp_permission_ids::cleanup::NO_MCP_SERVERS;

#[derive(Debug, Error)]
pub enum RequestedError {
    #[error("{field} entry {index} is not a string")]
    NotAString { field: &'static str, index: usize },
}

fn string_list(field: &'static str, raw: Option<&JsonData>) -> Result<Vec<String>, RequestedError> {
    let Some(JsonData::Array(items)) = raw else {
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

pub fn requested_server_ids(
    permission: Option<&JsonData>,
) -> Result<BTreeSet<String>, RequestedError> {
    let Some(JsonData::Object(fields)) = permission else {
        return Ok(BTreeSet::new());
    };
    let listed = string_list("mcp_servers", fields.get("mcp_servers"))?;
    let mut server_ids: BTreeSet<String> = listed.into_iter().collect();
    server_ids.remove(NO_MCP_SERVERS);
    if let Some(JsonData::Object(grants)) = fields.get("mcp_tool_permissions") {
        server_ids.extend(grants.keys().cloned());
    }
    Ok(server_ids)
}

pub fn requested_access_groups(
    permission: Option<&JsonData>,
) -> Result<BTreeSet<String>, RequestedError> {
    let Some(JsonData::Object(fields)) = permission else {
        return Ok(BTreeSet::new());
    };
    let listed = string_list("mcp_access_groups", fields.get("mcp_access_groups"))?;
    Ok(listed.into_iter().collect())
}

pub fn requested_toolsets(
    permission: Option<&JsonData>,
) -> Result<BTreeSet<String>, RequestedError> {
    let Some(JsonData::Object(fields)) = permission else {
        return Ok(BTreeSet::new());
    };
    let listed = string_list("mcp_toolsets", fields.get("mcp_toolsets"))?;
    Ok(listed.into_iter().collect())
}

pub fn requested_vector_stores(
    permission: Option<&JsonData>,
) -> Result<BTreeSet<String>, RequestedError> {
    let Some(JsonData::Object(fields)) = permission else {
        return Ok(BTreeSet::new());
    };
    let listed = string_list("vector_stores", fields.get("vector_stores"))?;
    Ok(listed
        .into_iter()
        .filter(|store| !store.is_empty())
        .collect())
}

pub fn requested_search_tools(
    permission: Option<&JsonData>,
) -> Result<Vec<String>, RequestedError> {
    let Some(JsonData::Object(fields)) = permission else {
        return Ok(Vec::new());
    };
    let listed = string_list("search_tools", fields.get("search_tools"))?;
    Ok(listed.into_iter().filter(|tool| !tool.is_empty()).collect())
}
