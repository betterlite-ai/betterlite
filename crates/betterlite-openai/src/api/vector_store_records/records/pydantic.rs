use super::timestamp::Timestamp;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreUpdateRequest {
    pub vector_store_id: String,
    pub custom_llm_provider: Option<String>,
    pub vector_store_name: Option<String>,
    pub vector_store_description: Option<String>,
    pub vector_store_metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreDeleteRequest {
    pub vector_store_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreInfoRequest {
    pub vector_store_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCreateLiteLlmParams {
    pub vector_store_index: String,
    pub vector_store_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCreateRequest {
    pub index_name: String,
    pub litellm_params: IndexCreateLiteLlmParams,
    pub index_info: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLlmManagedVectorStoreIndex {
    pub id: String,
    pub index_name: String,
    pub litellm_params: IndexCreateLiteLlmParams,
    pub index_info: Option<BTreeMap<String, Value>>,
    pub created_at: Option<Timestamp>,
    pub created_by: Option<String>,
    pub updated_at: Option<Timestamp>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IndexListObject {
    #[default]
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexListResponse {
    #[serde(default)]
    pub object: IndexListObject,
    pub data: Vec<LiteLlmManagedVectorStoreIndex>,
}
