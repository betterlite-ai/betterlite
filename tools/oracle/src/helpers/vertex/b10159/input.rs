use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

// The rawPredict reply the passthrough handler holds, passed to the partner config LiteLLM selects.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) publisher: String,
    pub(crate) model: String,
    pub(crate) status: u16,
    pub(crate) raw_text: String,
    pub(crate) headers: IndexMap<String, String>,
    pub(crate) messages: Vec<Json>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) request_data: IndexMap<String, Json>,
    pub(crate) json_mode: Option<bool>,
}
