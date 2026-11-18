use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

// one variant per Python function of the b5961 helper; the function key selects it
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ModelConfig(ModelConfig),
    Headers(Headers),
    HfRequest(HfRequest),
    HfReply(HfReply),
    CohereMap(CohereMap),
    CohereRequest(CohereRequest),
    CohereReply(CohereReply),
    DecodeStream(DecodeStream),
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelConfig {
    pub(crate) model: String,
}
// the caller's headers are the same input both LiteLLM methods drop
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Headers {
    pub(crate) kind: ConfigKind,
    pub(crate) headers: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfigKind {
    HuggingFace,
    Cohere,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HfRequest {
    pub(crate) input: Json,
    pub(crate) optional_params: Object,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HfReply {
    pub(crate) body: String,
    pub(crate) status: u16,
    pub(crate) model: String,
    pub(crate) request: Object,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohereMap {
    pub(crate) non_default: Object,
    pub(crate) optional_params: Object,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohereRequest {
    pub(crate) model: String,
    pub(crate) input: Json,
    pub(crate) optional_params: Object,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohereReply {
    pub(crate) model: String,
    pub(crate) body: String,
    pub(crate) input: Json,
    pub(crate) request_data: Object,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecodeStream {
    pub(crate) model: String,
    pub(crate) is_messages_api: bool,
    pub(crate) sync: bool,
    pub(crate) chunks: Vec<String>,
}
