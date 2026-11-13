use std::collections::BTreeMap;

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    model_cost: BTreeMap<String, BTreeMap<String, Value>>,
    chat_supported: Vec<ChatSupportedStep>,
    chat_map: Vec<ChatMapStep>,
    chat_messages: Vec<ChatMessagesStep>,
    completion: Vec<CompletionStep>,
    rerank_requests: Vec<RerankRequestStep>,
    rerank_responses: Vec<RerankResponseStep>,
    cost_categories: Vec<CategoryStep>,
    cost_registry: Vec<RegistryStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatSupportedStep {
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatMapStep {
    model: String,
    params: Object,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    drop_params: bool,
    #[serde(default)]
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatMessagesStep {
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionStep {
    model: String,
    messages: Vec<Value>,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RerankRequestStep {
    model: String,
    query: String,
    documents: Vec<RerankDocument>,
    #[serde(default)]
    top_n: Option<i64>,
    #[serde(default)]
    rank_fields: Option<Vec<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    return_documents: Option<Option<bool>>,
    #[serde(default)]
    max_chunks_per_doc: Option<i64>,
    api_key: String,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RerankResponseStep {
    response: Value,
    next_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CategoryStep {
    model: String,
    #[serde(default)]
    call_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryStep {
    model: String,
}
