use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_openai::files::records::CreatePurpose;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    TransformRecords(TransformRecords),
    CreateFileRequest(CreateFileRequest),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformRecords {
    pub(super) records: Vec<Value>,
    #[serde(default)]
    pub(super) target_model: String,
    #[serde(default)]
    pub(super) model_alias_map: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) model_cost: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFileRequest {
    pub(super) model: String,
    pub(super) file: FileInput,
    #[serde(default)]
    pub(super) purpose: Option<CreatePurpose>,
    #[serde(default)]
    pub(super) optional_params: Value,
    #[serde(default)]
    pub(super) litellm_params: Value,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) model_alias_map: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) model_cost: ParamMap,
    pub(super) uuid: String,
    pub(super) now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileInput {
    pub(super) content: String,
    #[serde(default)]
    pub(super) filename: Option<String>,
    #[serde(default)]
    pub(super) content_type: Option<String>,
}
