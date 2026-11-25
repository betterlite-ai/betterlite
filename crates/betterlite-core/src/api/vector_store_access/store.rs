use std::fmt::{self, Debug, Formatter};

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde::{Deserialize, Serialize};

use super::decode::parse_params;
use super::error::VectorStoreAccessError;

// LiteLLM_ManagedVectorStore inside this policy: the shared row, with a Debug that never prints a provider credential
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ManagedVectorStore(pub LiteLlmManagedVectorStore);

impl Debug for ManagedVectorStore {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // litellm_params and the extra passthrough carry provider credentials, so Debug names fields, not values
        let extra: Vec<&str> = self.0.extra.keys().map(String::as_str).collect();
        let params = match &self.0.litellm_params {
            Some(_) => "present",
            None => "absent",
        };
        f.debug_struct("ManagedVectorStore")
            .field("vector_store_id", &self.0.vector_store_id)
            .field("team_id", &self.0.team_id)
            .field("litellm_params", &params)
            .field("extra_fields", &extra)
            .finish_non_exhaustive()
    }
}

// _normalize_litellm_params: stored text becomes the object it encodes, or {} when it encodes none
pub fn normalize_litellm_params(
    vector_store: &ManagedVectorStore,
) -> Result<ManagedVectorStore, VectorStoreAccessError> {
    let record = &vector_store.0;
    let Some(params) = &record.litellm_params else {
        return Ok(vector_store.clone());
    };
    let Some(text) = params.as_str() else {
        return Ok(vector_store.clone());
    };
    let mut normalized = record.clone();
    normalized.litellm_params = Some(
        serde_json::to_value(Json::Object(parse_params(text.as_bytes())))
            .map_err(VectorStoreAccessError::Params)?,
    );
    Ok(ManagedVectorStore(normalized))
}
