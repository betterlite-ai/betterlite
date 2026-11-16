use std::collections::BTreeMap;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::fireworks_ai_common::SessionParams;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) catalog: Vec<CatalogModel>,
    #[serde(default)]
    pub(crate) headers: Vec<HeaderCall>,
    #[serde(default)]
    pub(crate) keys: Vec<KeyCall>,
    #[serde(default)]
    pub(crate) requests: Vec<RequestCall>,
    #[serde(default)]
    pub(crate) replies: Vec<ReplyCall>,
    #[serde(default)]
    pub(crate) chunks: Vec<ChunkCall>,
    #[serde(default)]
    pub(crate) models: Vec<ModelCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogModel {
    pub(crate) key: String,
    pub(crate) model_info: Box<RawValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderCall {
    #[serde(default)]
    pub(crate) headers: Headers,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) litellm_params: SessionParams,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyCall {
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<ChatMessage>,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkCall {
    pub(crate) chunk: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelCall {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
}
