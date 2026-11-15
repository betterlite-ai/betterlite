use std::collections::BTreeMap;

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model_cost: BTreeMap<String, BTreeMap<String, Value>>,
    pub(crate) chat_supported: Vec<ChatSupportedStep>,
    pub(crate) chat_map: Vec<ChatMapStep>,
    pub(crate) chat_messages: Vec<ChatMessagesStep>,
    pub(crate) completion: Vec<CompletionStep>,
    pub(crate) rerank_requests: Vec<RerankRequestStep>,
    pub(crate) rerank_responses: Vec<RerankResponseStep>,
    pub(crate) cost_categories: Vec<CategoryStep>,
    pub(crate) cost_registry: Vec<RegistryStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatSupportedStep {
    pub(crate) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatMapStep {
    pub(crate) model: String,
    pub(crate) params: Object,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatMessagesStep {
    pub(crate) messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionStep {
    pub(crate) model: String,
    pub(crate) messages: Vec<Value>,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RerankRequestStep {
    pub(crate) model: String,
    pub(crate) query: String,
    pub(crate) documents: Vec<RerankDocument>,
    #[serde(default)]
    pub(crate) top_n: Option<i64>,
    #[serde(default)]
    pub(crate) rank_fields: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) return_documents: ReturnDocumentsStep,
    #[serde(default)]
    pub(crate) max_chunks_per_doc: Option<i64>,
    pub(crate) api_key: String,
    pub(crate) api_base: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub(crate) enum ReturnDocumentsStep {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null,
    Flag(bool),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RerankResponseStep {
    pub(crate) response: Value,
    pub(crate) next_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CategoryStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) call_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryStep {
    pub(crate) model: String,
}
