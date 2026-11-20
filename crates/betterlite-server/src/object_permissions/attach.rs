use std::collections::BTreeMap;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;

use super::error::ObjectPermissionError;
use super::store::find_object_permission;

// ast-grep-ignore: no-long-names
pub async fn attach_object_permission_to_dict(
    data_dict: &mut BTreeMap<String, JsonData>,
    client: Option<PrismaClient<'_>>,
) -> Result<(), ObjectPermissionError> {
    let Some(client) = client else {
        return Err(ObjectPermissionError::PrismaClientMissing);
    };
    let Some(JsonData::String(object_permission_id)) = data_dict.get("object_permission_id") else {
        return Ok(());
    };
    if object_permission_id.is_empty() {
        return Ok(());
    }
    let found = find_object_permission(client, object_permission_id).await?;
    if let Some(row) = found {
        // The row dump replaces an attached object wholesale, the same value model_dump and the v1 dict fallback return.
        data_dict.insert("object_permission".to_owned(), row);
    }
    Ok(())
}
