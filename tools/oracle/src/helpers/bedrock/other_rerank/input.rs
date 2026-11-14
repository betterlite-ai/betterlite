use std::collections::BTreeMap;

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    TextAndJsonDocumentSource,
    RerankModelArn,
    TopN,
    ResultsIndexScore,
    ReturnDocumentContent,
    RerankError,
    RerankTransportLogging,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Supplied {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyFailure {
    Timeout,
    Failed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Credentials {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
    pub(crate) failure: Option<ReplyFailure>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) model: String,
    pub(crate) query: String,
    pub(crate) documents: Vec<RerankDocument>,
    pub(crate) top_n: Option<i64>,
    pub(crate) rank_fields: Option<Vec<String>>,
    pub(crate) return_documents: Option<bool>,
    pub(crate) max_chunks_per_doc: Option<i64>,
    pub(crate) optional_params: Map<String, Value>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) api_base: Option<String>,
    pub(crate) credentials: Credentials,
    pub(crate) reply: Reply,
    pub(crate) supplied: Supplied,
    pub(crate) is_async: bool,
    pub(crate) timeout_ms: Option<u64>,
}
