use std::collections::BTreeMap;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::server_record::MCPServer;

use crate::object_permissions::error::ObjectPermissionError;
use crate::object_permissions::update::handle_update_object_permission_common;

// LiteLLM's handle_update_object_permission: B816 writes the row and its id lands in the payload only when one comes back
pub async fn update_object_permission(
    data_json: &mut BTreeMap<String, JsonData>,
    existing_object_permission_id: Option<&str>,
    client: Option<PrismaClient<'_>>,
    registry: &[&MCPServer],
) -> Result<(), ObjectPermissionError> {
    let Some(object_permission_id) = handle_update_object_permission_common(
        data_json,
        existing_object_permission_id,
        client,
        registry,
    )
    .await?
    else {
        return Ok(());
    };
    data_json.insert(
        "object_permission_id".to_owned(),
        JsonData::String(object_permission_id),
    );
    Ok(())
}
