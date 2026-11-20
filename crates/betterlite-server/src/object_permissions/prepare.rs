use std::collections::BTreeMap;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::DEFAULT_MAX_RECURSE_DEPTH;
use betterlite_logging::private_json::safe_json::safe_dumps;
use betterlite_mcp::server_record::MCPServer;
use uuid::Uuid;

// ast-grep-ignore: no-long-names
use super::ambiguity::reject_ambiguous_mcp_tool_permission_keys;
use super::error::ObjectPermissionError;
use super::store::find_object_permission;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedUpsert {
    pub object_permission_id: String,
    pub record: BTreeMap<String, JsonData>,
}

// ast-grep-ignore: no-long-names
pub async fn prepare_object_permission_upsert(
    new_object_permission: &BTreeMap<String, JsonData>,
    existing_object_permission_id: Option<&str>,
    client: PrismaClient<'_>,
    registry: &[&MCPServer],
) -> Result<PreparedUpsert, ObjectPermissionError> {
    let object_permission_id = chosen_object_permission_id(existing_object_permission_id);
    let existing = find_object_permission(client, &object_permission_id).await?;
    let mut record = existing.as_ref().map(non_null_fields).unwrap_or_default();
    // ast-grep-ignore: no-long-names
    reject_ambiguous_mcp_tool_permission_keys(
        new_object_permission.get("mcp_tool_permissions"),
        record.get("mcp_tool_permissions"),
        Some(client),
        registry,
    )
    .await?;
    for (name, value) in new_object_permission {
        record.insert(name.clone(), value.clone());
    }
    record.insert(
        "object_permission_id".to_owned(),
        JsonData::String(object_permission_id.clone()),
    );
    if let Some(tools) = record.get("mcp_tool_permissions") {
        let serialized = serialized_tool_permissions(tools)?;
        record.insert("mcp_tool_permissions".to_owned(), serialized);
    }
    Ok(PreparedUpsert {
        object_permission_id,
        record,
    })
}

fn chosen_object_permission_id(existing: Option<&str>) -> String {
    match existing {
        Some(id) if !id.is_empty() => id.to_owned(),
        Some(_) | None => Uuid::new_v4().to_string(),
    }
}

// LiteLLM's exclude_unset and exclude_none dump of a stored row: every column it set that is not null.
pub(crate) fn non_null_fields(row: &JsonData) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = row else {
        return BTreeMap::new();
    };
    let mut kept = BTreeMap::new();
    for (name, value) in fields {
        if !matches!(value, JsonData::Null) {
            kept.insert(name.clone(), value.clone());
        }
    }
    kept
}

// A stored grant already is the column's JSON string; re-serializing it double-encodes the row and later updates fail.
pub(crate) fn serialized_tool_permissions(value: &JsonData) -> Result<JsonData, PrivateJsonError> {
    if let JsonData::String(stored) = value {
        return Ok(JsonData::String(stored.clone()));
    }
    safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None).map(JsonData::String)
}
