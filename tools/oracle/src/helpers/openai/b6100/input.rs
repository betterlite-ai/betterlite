use std::collections::BTreeMap;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::Value;

// one call carries one variant, so a case file names the operation it checks
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) costs: Vec<Value>,
    #[serde(default)]
    pub(crate) cost_bodies: Vec<String>,
    #[serde(default)]
    pub(crate) cost_headers: Vec<Headers>,
    #[serde(default)]
    pub(crate) credentials: Vec<Call>,
    #[serde(default)]
    pub(crate) chat_params: Vec<ChatParams>,
    #[serde(default)]
    pub(crate) chat_requests: Vec<ChatRequest>,
    #[serde(default)]
    pub(crate) chat_replies: Vec<String>,
    #[serde(default)]
    pub(crate) chunks: Vec<Value>,
    #[serde(default)]
    pub(crate) models: Vec<CatalogCase>,
    #[serde(default)]
    pub(crate) embedding_plans: Vec<EmbeddingPlan>,
    #[serde(default)]
    pub(crate) embedding_replies: Vec<String>,
    #[serde(default)]
    pub(crate) responses_prepare: Vec<Call>,
    #[serde(default)]
    pub(crate) responses_replies: Vec<String>,
    #[serde(default)]
    pub(crate) messages_environments: Vec<Call>,
    #[serde(default)]
    pub(crate) messages_replies: Vec<String>,
}

// the fields the credential helpers and the two header-building families read
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) global_api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatParams {
    pub(crate) model: String,
    pub(crate) reasoning: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatRequest {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogCase {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingPlan {
    pub(crate) model: String,
    pub(crate) input: EmbeddingInput,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) non_default_params: Object,
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) global_api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}
