use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::sdk_http::result::Failure;

// Which litellm entry point the case drives: completion, or the async one that restores session history first
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub(crate) mode: Mode,
    pub(crate) model: String,
    pub(crate) request: Json,
    pub(crate) api_key: String,
    pub(crate) timeout_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) extra_headers: Option<Json>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
    // LiteLLM_SpendLogs rows the injected repository answers, so the async path has history to restore
    #[serde(default)]
    pub(crate) history: Vec<Json>,
}

// What the streaming iterator names itself after, plus the event types it emitted in order
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamView {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) request_input: Json,
    pub(crate) responses_api_request: Json,
    pub(crate) litellm_metadata: Json,
    pub(crate) event_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub(crate) response: Option<Json>,
    pub(crate) stream: Option<StreamView>,
    pub(crate) failure: Option<Failure>,
    pub(crate) lookups: Vec<String>,
}
