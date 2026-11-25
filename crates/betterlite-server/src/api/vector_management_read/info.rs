use std::sync::PoisonError;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::pydantic::VectorStoreInfoRequest;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde::Serialize;

use super::caller::CallerView;
use super::error::ManagementReadError;
use super::state::VectorManagementReadState;
use crate::api::vector_management_boundaries::error::VectorManagementError;
use crate::api::vector_management_boundaries::fetch::{
    check_vector_store_access, fetch_store_before_access,
};
use crate::api::vector_management_boundaries::json::decode_management_json;
use crate::api::vector_management_boundaries::redaction::redact_litellm_params;
use crate::api::vector_management_boundaries::rows::VectorStoreRows;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InfoResponse {
    pub vector_store: StoreTable,
}

// LiteLLM_ManagedVectorStoresTable: all twelve fields, the null ones included, in the model's declared order
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoreTable {
    pub vector_store_id: String,
    pub custom_llm_provider: String,
    pub vector_store_name: Option<String>,
    pub vector_store_description: Option<String>,
    pub vector_store_metadata: Json,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub litellm_credential_name: Option<String>,
    pub litellm_params: Json,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub is_config: bool,
}

// get_vector_store_info: the feature switch, then the database check outside the try, then the catch-all
pub async fn get_vector_store_info<Database>(
    state: &VectorManagementReadState<Database>,
    caller: &CallerView,
    request: &VectorStoreInfoRequest,
) -> Result<InfoResponse, ManagementReadError>
where
    Database: VectorStoreRows + Send + Sync,
{
    state.check_vector_feature(caller).await?;
    let Some(rows) = state.database.as_ref() else {
        return Err(ManagementReadError::DatabaseNotConnected);
    };
    match cached_or_row(state, rows, caller, &request.vector_store_id).await {
        Ok(info) => Ok(info),
        Err(error) => {
            if !error.is_http_exception() {
                tracing::error!("Error getting vector store info: {error}");
            }
            Err(error)
        }
    }
}

async fn cached_or_row<Database>(
    state: &VectorManagementReadState<Database>,
    rows: &Database,
    caller: &CallerView,
    vector_store_id: &str,
) -> Result<InfoResponse, ManagementReadError>
where
    Database: VectorStoreRows + Send + Sync,
{
    let cached = state.registry.as_ref().and_then(|registry| {
        registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store_by_id(vector_store_id)
            .cloned()
    });
    if let Some(store) = cached {
        return authorized_cached(state, caller, &store).await;
    }
    let store =
        fetch_store_before_access(rows, &state.access(), vector_store_id, &caller.key).await?;
    row_info(&store, state.limits)
}

async fn authorized_cached<Database>(
    state: &VectorManagementReadState<Database>,
    caller: &CallerView,
    store: &LiteLlmManagedVectorStore,
) -> Result<InfoResponse, ManagementReadError> {
    let allowed = check_vector_store_access(&state.access(), store, &caller.key)
        .await
        .map_err(VectorManagementError::Access)
        .map_err(ManagementReadError::Management)?;
    if !allowed {
        return Err(ManagementReadError::Management(
            VectorManagementError::AccessDenied,
        ));
    }
    Ok(cached_info(store, state.limits)?)
}

// the registry branch: blanks collapse to null, the credential name does not, and metadata text is parsed
fn cached_info(
    store: &LiteLlmManagedVectorStore,
    limits: ApiLimits,
) -> Result<InfoResponse, ManagementReadError> {
    let table = StoreTable {
        vector_store_id: store.vector_store_id.clone().unwrap_or_default(),
        custom_llm_provider: store.custom_llm_provider.clone().unwrap_or_default(),
        vector_store_name: nonempty(store.vector_store_name.as_deref()),
        vector_store_description: nonempty(store.vector_store_description.as_deref()),
        vector_store_metadata: cached_metadata(store, limits)?,
        created_at: nonempty(store.created_at.as_deref()),
        updated_at: nonempty(store.updated_at.as_deref()),
        litellm_credential_name: store.litellm_credential_name.clone(),
        litellm_params: redacted_params(store, limits)?,
        team_id: nonempty(store.team_id.as_deref()),
        user_id: nonempty(store.user_id.as_deref()),
        is_config: store.is_config.unwrap_or(false),
    };
    Ok(InfoResponse {
        vector_store: table,
    })
}

// the database branch: the response model over the full row, so blanks and nulls stay exactly as stored
fn row_info(
    store: &LiteLlmManagedVectorStore,
    limits: ApiLimits,
) -> Result<InfoResponse, ManagementReadError> {
    let metadata = match &store.vector_store_metadata {
        Some(value) => serde_json::from_value(value.clone())?,
        None => Json::Null,
    };
    let table = StoreTable {
        vector_store_id: store.vector_store_id.clone().unwrap_or_default(),
        custom_llm_provider: store.custom_llm_provider.clone().unwrap_or_default(),
        vector_store_name: store.vector_store_name.clone(),
        vector_store_description: store.vector_store_description.clone(),
        vector_store_metadata: metadata,
        created_at: store.created_at.clone(),
        updated_at: store.updated_at.clone(),
        litellm_credential_name: store.litellm_credential_name.clone(),
        litellm_params: redacted_params(store, limits)?,
        team_id: store.team_id.clone(),
        user_id: store.user_id.clone(),
        is_config: store.is_config.unwrap_or(false),
    };
    Ok(InfoResponse {
        vector_store: table,
    })
}

fn nonempty(value: Option<&str>) -> Option<String> {
    let text = value?;
    if text.is_empty() {
        return None;
    }
    Some(text.to_owned())
}

fn redacted_params(
    store: &LiteLlmManagedVectorStore,
    limits: ApiLimits,
) -> Result<Json, ManagementReadError> {
    let Some(value) = &store.litellm_params else {
        return Ok(Json::Null);
    };
    let stored: Json = serde_json::from_value(value.clone())?;
    Ok(redact_litellm_params(&stored, 0, limits))
}

// a JSON string metadata is json.loads'd; a parsed object is kept, null is kept, any other parse is refused
fn cached_metadata(
    store: &LiteLlmManagedVectorStore,
    limits: ApiLimits,
) -> Result<Json, ManagementReadError> {
    let Some(value) = &store.vector_store_metadata else {
        return Ok(Json::Null);
    };
    let metadata: Json = serde_json::from_value(value.clone())?;
    match metadata {
        Json::Text(text) => parsed_metadata(&text, limits),
        object @ Json::Object(_) => Ok(object),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => Ok(Json::Null),
    }
}

fn parsed_metadata(text: &str, limits: ApiLimits) -> Result<Json, ManagementReadError> {
    let decoded = decode_management_json(text.as_bytes(), limits)?;
    match decoded {
        Json::Null => Ok(Json::Null),
        object @ Json::Object(_) => Ok(object),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Text(_) => {
            Err(ManagementReadError::MetadataShape)
        }
    }
}
