use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde::Serialize;

use super::error::ManagementWriteError;
use crate::api::vector_management_boundaries::redaction::redact_store_params;

pub const SUCCESS: &str = "success";

// {"status": "success", "message": ..., "vector_store": <the redacted row>}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorStoreWriteResponse {
    pub status: String,
    pub message: String,
    pub vector_store: ManagedVectorStoreRow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorStoreDeleteResponse {
    pub status: String,
    pub message: String,
}

impl VectorStoreDeleteResponse {
    #[must_use]
    pub fn deleted(vector_store_id: &str) -> Self {
        Self {
            status: SUCCESS.to_owned(),
            message: format!("Vector store {vector_store_id} deleted successfully"),
        }
    }
}

impl VectorStoreWriteResponse {
    // the DB row as stored, with litellm_params redacted through B2486 before it leaves
    pub fn success(
        message: String,
        store: &LiteLlmManagedVectorStore,
        limits: ApiLimits,
    ) -> Result<Self, ManagementWriteError> {
        let redacted = redact_store_params(store, limits)?;
        Ok(Self {
            status: SUCCESS.to_owned(),
            message,
            vector_store: ManagedVectorStoreRow::from_record(&redacted)?,
        })
    }
}

// LiteLLM_ManagedVectorStoresTable as the write routes echo it: the eleven row fields, no is_config
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedVectorStoreRow {
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
}

impl ManagedVectorStoreRow {
    fn from_record(record: &LiteLlmManagedVectorStore) -> Result<Self, ManagementWriteError> {
        Ok(Self {
            vector_store_id: record.vector_store_id.clone().unwrap_or_default(),
            custom_llm_provider: record.custom_llm_provider.clone().unwrap_or_default(),
            vector_store_name: record.vector_store_name.clone(),
            vector_store_description: record.vector_store_description.clone(),
            vector_store_metadata: decode_json(record.vector_store_metadata.as_ref())?,
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
            litellm_credential_name: record.litellm_credential_name.clone(),
            litellm_params: decode_json(record.litellm_params.as_ref())?,
            team_id: record.team_id.clone(),
            user_id: record.user_id.clone(),
        })
    }
}

// the record field's JSON value as the decision tree the routes render; absent is null
pub fn decode_json<T>(value: Option<&T>) -> Result<Json, ManagementWriteError>
where
    T: Serialize,
{
    let Some(value) = value else {
        return Ok(Json::Null);
    };
    let text = serde_json::to_string(value).map_err(ManagementWriteError::Json)?;
    serde_json::from_str(&text).map_err(ManagementWriteError::Json)
}
