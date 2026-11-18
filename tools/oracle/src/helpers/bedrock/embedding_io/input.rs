use std::collections::BTreeMap;

use serde::Deserialize;

use betterlite_prompts::arguments::json::Object;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SignedEmbeddingRequest(SignedRequest),
    ProviderJsonResponse(HttpRequest),
    EmbeddingUsage(TransformRequest),
    HttpError(HttpRequest),
    EmbeddingCredentials(CredentialRequest),
    AsyncEmbeddingSingleArn(TransformRequest),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedRequest {
    pub(crate) credentials: Keys,
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) region: String,
    pub(crate) now: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    #[serde(default)]
    pub(crate) session_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HttpRequest {
    pub(crate) asynchronous: bool,
    pub(crate) supplied: Supplied,
    pub(crate) url: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) data: Object,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) timeout_record: Option<TimeoutRecord>,
    pub(crate) reply: Reply,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Supplied {
    None,
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimeoutRecord {
    pub(crate) connect: f64,
    pub(crate) read: f64,
    pub(crate) write: f64,
    pub(crate) pool: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) failure: Option<Failure>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Timeout,
    Failed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformRequest {
    pub(crate) model: String,
    pub(crate) provider: Provider,
    pub(crate) response_list: Vec<Object>,
    #[serde(default)]
    pub(crate) batch_data: Option<Vec<Object>>,
    #[serde(default)]
    pub(crate) is_async_invoke: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Provider {
    Amazon,
    Cohere,
    Nova,
    Twelvelabs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialRequest {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) bearer_token: Option<String>,
    pub(crate) credentials: Keys,
}
