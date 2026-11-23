use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::threads::records::ThreadMessageData;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::error::CallError;

mod tests;

// litellm.api_key and the other module globals _records.py reads before every call.
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ModuleSettings {
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) openai_key: Option<String>,
    pub(crate) azure_key: Option<String>,
}

// One create_thread or get_thread call: the named arguments next to the caller's kwargs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThreadRequest {
    pub(crate) custom_llm_provider: String,
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    #[serde(default)]
    pub(crate) messages: Option<Vec<ThreadMessageData>>,
    #[serde(default)]
    pub(crate) metadata: Option<BTreeMap<String, Json>>,
    #[serde(default)]
    pub(crate) tool_resources: Option<Json>,
    #[serde(default)]
    pub(crate) kwargs: BTreeMap<String, Json>,
}

pub(crate) fn read<T>(field: &'static str, text: &str) -> Result<T, CallError>
where
    T: DeserializeOwned,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(CallError::Json { field, source }),
    }
}
