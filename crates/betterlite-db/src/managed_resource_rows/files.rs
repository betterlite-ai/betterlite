use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::JsonData;

use super::wire::RowTimestamp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedFileTable<File = JsonData> {
    pub unified_file_id: String,
    #[serde(default = "default_file_object")]
    pub file_object: Option<File>,
    pub model_mappings: BTreeMap<String, String>,
    pub flat_model_file_ids: Vec<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub storage_backend: Option<String>,
    #[serde(default)]
    pub storage_url: Option<String>,
}

#[must_use]
pub const fn default_file_object<File>() -> Option<File> {
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagedObjectPurpose {
    #[serde(rename = "batch")]
    Batch,
    #[serde(rename = "fine-tune")]
    FineTune,
    #[serde(rename = "response")]
    Response,
    #[serde(rename = "container")]
    Container,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ManagedObjectBody<Batch = JsonData, FineTuning = JsonData, Response = JsonData> {
    Batch(Box<Batch>),
    FineTuning(Box<FineTuning>),
    Response(Box<Response>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedObjectTable<Batch = JsonData, FineTuning = JsonData, Response = JsonData> {
    pub unified_object_id: String,
    pub model_object_id: String,
    pub file_purpose: ManagedObjectPurpose,
    pub file_object: ManagedObjectBody<Batch, FineTuning, Response>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub org_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedVectorStoreTable {
    pub unified_resource_id: String,
    #[serde(default)]
    pub resource_object: Option<JsonData>,
    pub model_mappings: BTreeMap<String, String>,
    pub flat_model_resource_ids: Vec<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub storage_backend: Option<String>,
    #[serde(default)]
    pub storage_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedVectorStoresTable {
    pub vector_store_id: String,
    pub custom_llm_provider: String,
    #[serde(default)]
    pub vector_store_name: Option<String>,
    #[serde(default)]
    pub vector_store_description: Option<String>,
    #[serde(default)]
    pub vector_store_metadata: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub created_at: Option<RowTimestamp>,
    #[serde(default)]
    pub updated_at: Option<RowTimestamp>,
    #[serde(default)]
    pub litellm_credential_name: Option<String>,
    #[serde(default)]
    pub litellm_params: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub is_config: bool,
}
