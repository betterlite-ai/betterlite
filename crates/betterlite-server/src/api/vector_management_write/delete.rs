use betterlite_openai::api::vector_store_records::records::pydantic::VectorStoreDeleteRequest;

use super::error::ManagementWriteError;
use super::response::VectorStoreDeleteResponse;
use super::rows::VectorStoreWrites;
use super::state::VectorManagementWriteState;
use crate::api::vector_management_boundaries::error::VectorManagementError;
use crate::api::vector_management_boundaries::fetch::check_vector_store_access;
use crate::api::vector_management_boundaries::rows::VectorStoreRows;
use crate::api::vector_management_read::caller::CallerView;

// delete_vector_store: the feature switch, then the database check outside the try, then the catch-all
pub async fn delete_vector_store<Database>(
    state: &VectorManagementWriteState<Database>,
    caller: &CallerView,
    request: &VectorStoreDeleteRequest,
) -> Result<VectorStoreDeleteResponse, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync,
{
    state.reads.check_vector_feature(caller).await?;
    let Some(database) = state.reads.database.as_ref() else {
        return Err(ManagementWriteError::DatabaseNotConnected);
    };
    match delete_route(state, database, caller, &request.vector_store_id).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if !error.is_http_exception() {
                tracing::error!("Error deleting vector store: {error}");
            }
            Err(error)
        }
    }
}

async fn delete_route<Database>(
    state: &VectorManagementWriteState<Database>,
    database: &Database,
    caller: &CallerView,
    vector_store_id: &str,
) -> Result<VectorStoreDeleteResponse, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync,
{
    let row = VectorStoreRows::find_unique(database, vector_store_id)
        .await
        .map_err(VectorManagementError::Rows)?;
    let memory = state
        .cache
        .as_ref()
        .and_then(|cache| cache.find(vector_store_id));
    let Some(check) = row.clone().or_else(|| memory.clone()) else {
        return Err(ManagementWriteError::Management(
            VectorManagementError::NotFound {
                vector_store_id: vector_store_id.to_owned(),
            },
        ));
    };
    state.refuse_config_owned(vector_store_id)?;
    let allowed = check_vector_store_access(&state.reads.access(), &check, &caller.key)
        .await
        .map_err(VectorManagementError::Access)?;
    if !allowed {
        return Err(ManagementWriteError::DeleteAccessDenied);
    }
    if row.is_some() {
        let _deleted = VectorStoreWrites::delete(database, vector_store_id)
            .await
            .map_err(VectorManagementError::Rows)?;
    }
    if memory.is_some()
        && let Some(registry) = state.cache.as_ref()
    {
        registry.remove(vector_store_id)?;
    }
    Ok(VectorStoreDeleteResponse::deleted(vector_store_id))
}
