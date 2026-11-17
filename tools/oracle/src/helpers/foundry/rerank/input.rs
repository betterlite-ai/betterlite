use std::collections::BTreeMap;

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) urls: Vec<UrlStep>,
    #[serde(default)]
    pub(super) requests: Vec<RequestStep>,
    #[serde(default)]
    pub(super) replies: Vec<ReplyStep>,
    #[serde(default)]
    pub(super) auths: Vec<AuthStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UrlStep {
    pub(super) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestStep {
    pub(super) model: String,
    pub(super) query: String,
    #[serde(default)]
    pub(super) documents: Vec<RerankDocument>,
    pub(super) top_n: Option<i64>,
    pub(super) rank_fields: Option<Vec<String>>,
    pub(super) return_documents: Option<bool>,
    pub(super) max_chunks_per_doc: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyStep {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) model_group: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthStep {
    pub(super) api_key: Option<String>,
    pub(super) global_api_key: Option<String>,
    pub(super) params: Option<Object>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}
