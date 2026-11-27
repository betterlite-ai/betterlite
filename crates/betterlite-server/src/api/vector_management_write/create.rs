use betterlite_auth::virtual_key_routes::caller::RouteCaller;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use super::error::ManagementWriteError;
use super::params::generic_params_dump;
use super::response::{VectorStoreWriteResponse, decode_json};
use super::rows::VectorStoreWrites;
use super::state::VectorManagementWriteState;
use super::{NewVectorStore, create_vector_store_in_db};
use crate::api::vector_management_read::caller::CallerView;

// new_vector_store: the feature switch, then the try block's catch-all
pub async fn new_vector_store<Database>(
    state: &VectorManagementWriteState<Database>,
    caller: &CallerView,
    body: &LiteLlmManagedVectorStore,
) -> Result<VectorStoreWriteResponse, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync,
{
    state.reads.check_vector_feature(caller).await?;
    match create_route(state, caller, body).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if !error.is_http_exception() {
                tracing::error!("Error creating vector store: {error}");
            }
            Err(error)
        }
    }
}

async fn create_route<Database>(
    state: &VectorManagementWriteState<Database>,
    caller: &CallerView,
    body: &LiteLlmManagedVectorStore,
) -> Result<VectorStoreWriteResponse, ManagementWriteError>
where
    Database: VectorStoreWrites + Send + Sync,
{
    let Some(vector_store_id) = body.vector_store_id.clone().filter(|id| !id.is_empty()) else {
        return Err(ManagementWriteError::RequiredFields);
    };
    let Some(custom_llm_provider) = body
        .custom_llm_provider
        .clone()
        .filter(|provider| !provider.is_empty())
    else {
        return Err(ManagementWriteError::RequiredFields);
    };
    state.refuse_config_owned(&vector_store_id)?;
    let request = NewVectorStore {
        vector_store_id: vector_store_id.clone(),
        custom_llm_provider,
        vector_store_name: body.vector_store_name.clone(),
        vector_store_description: body.vector_store_description.clone(),
        vector_store_metadata: metadata_value(body)?,
        litellm_params: params_value(body)?,
        litellm_credential_name: body.litellm_credential_name.clone(),
        team_id: RouteCaller::of_key(&caller.key).team_id.map(str::to_owned),
        user_id: caller.user_id.clone(),
    };
    let created = create_vector_store_in_db(
        state.reads.database.as_ref(),
        state.cache.as_deref(),
        request,
    )
    .await?;
    VectorStoreWriteResponse::success(
        format!("Vector store {vector_store_id} created successfully"),
        &created,
        state.reads.limits,
    )
}

// metadata is kept only when it parsed as a JSON object; a string is dropped as LiteLLM drops it
fn metadata_value(body: &LiteLlmManagedVectorStore) -> Result<Option<Json>, ManagementWriteError> {
    match decode_json(body.vector_store_metadata.as_ref())? {
        object @ Json::Object(_) => Ok(Some(object)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Ok(None),
    }
}

fn params_value(
    body: &LiteLlmManagedVectorStore,
) -> Result<Option<JsonObject>, ManagementWriteError> {
    match decode_json(body.litellm_params.as_ref())? {
        Json::Object(fields) => Ok(Some(fields)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Ok(None),
    }
}

pub(super) fn optional_metadata_text(
    field: Option<&Json>,
) -> Result<Option<String>, ManagementWriteError> {
    let Some(metadata) = field else {
        return Ok(None);
    };
    safe_dumps(metadata, DEFAULT_MAX_RECURSE_DEPTH, None)
        .map(Some)
        .map_err(ManagementWriteError::PrivateJson)
}

// an absent or empty dict stores {}, any other stores GenericLiteLLMParams.model_dump(exclude_none=True)
pub(super) fn params_text(field: Option<&JsonObject>) -> Result<String, ManagementWriteError> {
    let empty = JsonObject::new();
    let Some(params) = field.filter(|candidate| !candidate.is_empty()) else {
        return safe_dumps(&empty, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(ManagementWriteError::PrivateJson);
    };
    let dumped = generic_params_dump(params)?;
    safe_dumps(&dumped, DEFAULT_MAX_RECURSE_DEPTH, None).map_err(ManagementWriteError::PrivateJson)
}
