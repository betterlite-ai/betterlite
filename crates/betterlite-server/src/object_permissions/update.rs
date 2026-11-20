use std::collections::BTreeMap;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_repository::permission::ObjectPermissionRepository;
use betterlite_db::resource_repository::table::RowWrite;
use betterlite_mcp::server_record::MCPServer;
use betterlite_redis::config_sync::table_actions::TableActions as _;

// ast-grep-ignore: no-long-names
use super::ambiguity::reject_ambiguous_mcp_tool_permission_keys;
use super::decode_object_permission;
use super::error::ObjectPermissionError;
use super::prepare::PreparedUpsert;
// ast-grep-ignore: no-long-names
use super::prepare::prepare_object_permission_upsert;
use super::prepare::serialized_tool_permissions;

// ast-grep-ignore: no-long-names
pub async fn handle_update_object_permission_common(
    data_json: &mut BTreeMap<String, JsonData>,
    existing_object_permission_id: Option<&str>,
    client: Option<PrismaClient<'_>>,
    registry: &[&MCPServer],
) -> Result<Option<String>, ObjectPermissionError> {
    let Some(client) = client else {
        return Err(ObjectPermissionError::PrismaClientMissing);
    };
    let Some(new_object_permission) = data_json.remove("object_permission") else {
        return Ok(None);
    };
    if matches!(new_object_permission, JsonData::Null) {
        return Ok(None);
    }
    let fields = permission_fields(new_object_permission)?;
    let prepared =
        // ast-grep-ignore: no-long-names
        prepare_object_permission_upsert(&fields, existing_object_permission_id, client, registry)
            .await?;
    let PreparedUpsert {
        object_permission_id,
        record,
    } = prepared;
    let row = ObjectPermissionRepository::new(Some(client), true)
        .table()
        .map_err(TableError::Database)?
        .upsert(object_permission_id, RowWrite(record))
        .await?;
    tracing::debug!(row = ?row, "created_object_permission_row");
    object_permission_id_of(&row).map(Some)
}

// A valid non-object value, whether it arrived as JSON or as a JSON string, becomes an empty record.
fn permission_fields(value: JsonData) -> Result<BTreeMap<String, JsonData>, ObjectPermissionError> {
    match value {
        JsonData::Object(fields) => Ok(fields),
        JsonData::String(text) => {
            let JsonData::Object(fields) = decode_object_permission(text.as_bytes())?.json else {
                return Ok(BTreeMap::new());
            };
            Ok(fields)
        }
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => {
            Ok(BTreeMap::new())
        }
    }
}

pub(crate) fn object_permission_id_of(row: &JsonData) -> Result<String, ObjectPermissionError> {
    let JsonData::Object(fields) = row else {
        return Err(ObjectPermissionError::MissingObjectPermissionId);
    };
    match fields.get("object_permission_id") {
        Some(JsonData::String(id)) => Ok(id.clone()),
        None
        | Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        ) => Err(ObjectPermissionError::MissingObjectPermissionId),
    }
}

// Creation drops null fields and a caller id, serializes the tool map, and changes the input only after the insert.
pub async fn set_object_permission(
    data_json: &mut BTreeMap<String, JsonData>,
    client: Option<PrismaClient<'_>>,
    registry: &[&MCPServer],
) -> Result<(), ObjectPermissionError> {
    let Some(client) = client else {
        return Ok(());
    };
    let Some(value) = data_json.get("object_permission").cloned() else {
        return Ok(());
    };
    let JsonData::Object(permission) = value else {
        data_json.remove("object_permission");
        return Ok(());
    };
    let mut clean = clean_permission_fields(&permission);
    // ast-grep-ignore: no-long-names
    reject_ambiguous_mcp_tool_permission_keys(
        clean.get("mcp_tool_permissions"),
        None,
        Some(client),
        registry,
    )
    .await?;
    if let Some(tools) = clean.get("mcp_tool_permissions").cloned() {
        clean.insert(
            "mcp_tool_permissions".to_owned(),
            serialized_tool_permissions(&tools)?,
        );
    }
    let created = ObjectPermissionRepository::new(Some(client), true)
        .table()
        .map_err(TableError::Database)?
        .create(RowWrite(clean))
        .await?;
    let created_id = object_permission_id_of(&created)?;
    data_json.insert(
        "object_permission_id".to_owned(),
        JsonData::String(created_id),
    );
    data_json.remove("object_permission");
    Ok(())
}

fn clean_permission_fields(permission: &BTreeMap<String, JsonData>) -> BTreeMap<String, JsonData> {
    let mut clean = BTreeMap::new();
    for (name, value) in permission {
        if name.as_str() != "object_permission_id" && !matches!(value, JsonData::Null) {
            clean.insert(name.clone(), value.clone());
        }
    }
    clean
}
