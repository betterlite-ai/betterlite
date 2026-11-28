use std::collections::BTreeMap;

use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::pydantic::VectorStoreUpdateRequest;
use serde::Deserialize;

use super::error::ManagementWriteError;
use super::response::VectorStoreWriteResponse;
use super::rows::{FieldChange, VectorStoreChanges, VectorStoreWrites};
use super::state::VectorManagementWriteState;
use crate::api::vector_management_boundaries::error::VectorManagementError;
use crate::api::vector_management_boundaries::fetch::fetch_store_before_access;
use crate::api::vector_management_read::caller::CallerView;

// model_dump(exclude_unset=True): absent, explicit null and a value stay apart, and the id never arrives here
#[derive(Debug, Deserialize)]
struct UpdateFields {
    #[serde(default, with = "serde_with::rust::double_option")]
    custom_llm_provider: FieldChange<String>,
    #[serde(default, with = "serde_with::rust::double_option")]
    vector_store_name: FieldChange<String>,
    #[serde(default, with = "serde_with::rust::double_option")]
    vector_store_description: FieldChange<String>,
    #[serde(default, with = "serde_with::rust::double_option")]
    vector_store_metadata: FieldChange<BTreeMap<String, Json>>,
}

// update_vector_store: the feature switch, then the try block's catch-all
pub async fn update_vector_store<Database>(
    state: &VectorManagementWriteState<Database>,
    caller: &CallerView,
    body: &VectorStoreUpdateRequest,
    raw: &[u8],
) -> Result<VectorStoreWriteResponse, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync,
{
    state.reads.check_vector_feature(caller).await?;
    let Some(database) = state.reads.database.as_ref() else {
        return Err(ManagementWriteError::DatabaseNotConnected);
    };
    match update_route(state, database, caller, body, raw).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if !error.is_http_exception() {
                tracing::error!("Error updating vector store: {error}");
            }
            Err(error)
        }
    }
}

async fn update_route<Database>(
    state: &VectorManagementWriteState<Database>,
    database: &Database,
    caller: &CallerView,
    body: &VectorStoreUpdateRequest,
    raw: &[u8],
) -> Result<VectorStoreWriteResponse, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync,
{
    let vector_store_id = body.vector_store_id.clone();
    state.refuse_config_owned(&vector_store_id)?;
    let _found = fetch_store_before_access(
        database,
        &state.reads.access(),
        &vector_store_id,
        &caller.key,
    )
    .await?;
    let fields: UpdateFields = serde_json::from_slice(raw).map_err(ManagementWriteError::Json)?;
    let changes = VectorStoreChanges {
        custom_llm_provider: fields.custom_llm_provider,
        vector_store_name: fields.vector_store_name,
        vector_store_description: fields.vector_store_description,
        vector_store_metadata: metadata_change(fields.vector_store_metadata)?,
    };
    let stored = VectorStoreWrites::update(database, &vector_store_id, changes)
        .await
        .map_err(VectorManagementError::Rows)?;
    let Some(updated) = stored else {
        return Err(ManagementWriteError::Management(
            VectorManagementError::NotFound {
                vector_store_id: vector_store_id.clone(),
            },
        ));
    };
    if let Some(cache) = state.cache.as_ref() {
        cache.update(&vector_store_id, updated.clone())?;
        tracing::debug!(
            "Updated vector store {vector_store_id} in both database and in-memory registry"
        );
    }
    VectorStoreWriteResponse::success(
        format!("Vector store {vector_store_id} updated successfully"),
        &updated,
        state.reads.limits,
    )
}

// a stored metadata dict is JSON text; an explicit null stays null
fn metadata_change(
    field: FieldChange<BTreeMap<String, Json>>,
) -> Result<FieldChange<String>, ManagementWriteError> {
    match field {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(metadata)) => {
            let text = safe_dumps(&metadata, DEFAULT_MAX_RECURSE_DEPTH, None)?;
            Ok(Some(Some(text)))
        }
    }
}
