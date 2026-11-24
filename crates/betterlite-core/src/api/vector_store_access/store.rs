use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use super::decode::parse_params;
use super::error::VectorStoreAccessError;

// _normalize_litellm_params: stored text becomes the object it encodes, or {} when it encodes none
pub fn normalize_litellm_params(
    vector_store: &LiteLlmManagedVectorStore,
) -> Result<LiteLlmManagedVectorStore, VectorStoreAccessError> {
    let Some(params) = &vector_store.litellm_params else {
        return Ok(vector_store.clone());
    };
    let Some(text) = params.as_str() else {
        return Ok(vector_store.clone());
    };
    let mut normalized = vector_store.clone();
    normalized.litellm_params = Some(
        serde_json::to_value(Json::Object(parse_params(text.as_bytes())))
            .map_err(VectorStoreAccessError::Params)?,
    );
    Ok(normalized)
}
