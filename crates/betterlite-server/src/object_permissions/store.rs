use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_repository::permission::ObjectPermissionRepository;
use betterlite_db::resource_repository::table::KeyFilter;

// The unique read both attach and prepare run: one row by its primary key, through the read client.
pub(crate) async fn find_object_permission(
    client: PrismaClient<'_>,
    object_permission_id: &str,
) -> Result<Option<JsonData>, TableError> {
    let table = ObjectPermissionRepository::new(Some(client), false)
        .table()
        .map_err(TableError::Database)?;
    let rows = table
        .as_ref()
        .find::<JsonData>(KeyFilter::Equal(object_permission_id))
        .await?;
    Ok(rows.into_iter().next())
}
