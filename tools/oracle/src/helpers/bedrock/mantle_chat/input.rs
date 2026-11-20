use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as MessageJson;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ChatUrl(UrlCases),
    MessageUrl(UrlCases),
    MessagesRequest(MessagesCall),
    ChatRequest(ChatCall),
    NativeSse(NativeSseCall),
    MessagesResponse(ResponseCall),
    RemoteUrlInlined(ChatCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCases {
    pub(crate) calls: Vec<UrlCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) headers: IndexMap<String, String>,
    #[serde(default)]
    pub(crate) metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) policy_fields: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesCall {
    pub(crate) model_cost: ParamMap,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<MessageJson>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    // The parent reads the caller's beta header from this same dict the override later rewrites.
    #[serde(default)]
    pub(crate) request_headers: IndexMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatCall {
    pub(crate) model_cost: ParamMap,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) optional_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) fetch: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeSseCall {
    pub(crate) events: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    pub(crate) body: String,
    #[serde(default = "default_status")]
    pub(crate) status: u16,
}

const fn default_status() -> u16 {
    200
}
