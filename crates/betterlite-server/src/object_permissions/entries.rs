use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use thiserror::Error;

pub type ToolPermissionEntries = BTreeMap<String, BTreeSet<String>>;

#[derive(Debug, Error)]
pub enum ToolPermissionEntriesError {
    #[error("mcp_tool_permissions is not valid JSON")]
    Bounded(#[from] PrivateJsonError),
    #[error("mcp_tool_permissions could not be rebuilt as JSON")]
    Document(#[source] serde_json::Error),
    #[error("mcp_tool_permissions is not a JSON object")]
    NotAnObject,
    #[error("mcp_tool_permissions entry '{identifier}' is not a list of tool names or null")]
    Value { identifier: String },
}

// A JSON string is parsed, a mapping validated, and every other raw type (null, number, array) is an empty grant.
pub fn mcp_tool_permission_entries(
    raw: Option<&JsonData>,
) -> Result<ToolPermissionEntries, ToolPermissionEntriesError> {
    match raw {
        None
        | Some(JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_)) => {
            Ok(BTreeMap::new())
        }
        Some(JsonData::Object(fields)) => entries_from_object(fields),
        Some(JsonData::String(text)) => entries_from_text(text),
    }
}

fn entries_from_text(text: &str) -> Result<ToolPermissionEntries, ToolPermissionEntriesError> {
    let loaded = safe_json_loads(text.as_bytes())?;
    let value =
        serde_json::from_value::<JsonData>(loaded).map_err(ToolPermissionEntriesError::Document)?;
    let JsonData::Object(fields) = value else {
        return Err(ToolPermissionEntriesError::NotAnObject);
    };
    entries_from_object(&fields)
}

fn entries_from_object(
    fields: &BTreeMap<String, JsonData>,
) -> Result<ToolPermissionEntries, ToolPermissionEntriesError> {
    let mut entries = BTreeMap::new();
    for (identifier, tools) in fields {
        let items: &[JsonData] = match tools {
            JsonData::Null => &[],
            JsonData::Array(array) => array.as_slice(),
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Object(_) => {
                return Err(ToolPermissionEntriesError::Value {
                    identifier: identifier.clone(),
                });
            }
        };
        let mut names = BTreeSet::new();
        for item in items {
            let JsonData::String(name) = item else {
                return Err(ToolPermissionEntriesError::Value {
                    identifier: identifier.clone(),
                });
            };
            names.insert(name.clone());
        }
        entries.insert(identifier.clone(), names);
    }
    Ok(entries)
}
