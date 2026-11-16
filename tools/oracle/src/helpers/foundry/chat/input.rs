use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) model_cost: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) chat_models: Vec<String>,
    #[serde(default)]
    pub(crate) text_models: Vec<String>,
    #[serde(default)]
    pub(crate) embedding_models: Vec<String>,
    pub(crate) urls: Option<Vec<UrlCall>>,
    pub(crate) supported: Option<Vec<String>>,
    pub(crate) maps: Option<Vec<MapCall>>,
    pub(crate) requests: Option<Vec<RequestCall>>,
    pub(crate) responses: Option<Vec<ResponseCall>>,
    pub(crate) providers: Option<Vec<ProviderCall>>,
    pub(crate) auths: Option<Vec<AuthCall>>,
    pub(crate) fields: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCall {
    pub(crate) model: String,
    pub(crate) non_default: Object,
    pub(crate) optional: Object,
    pub(crate) drop_params: bool,
    pub(crate) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) optional: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    pub(crate) model: String,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCall {
    pub(crate) model: String,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthCall {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) litellm_params: Object,
    pub(crate) environment: BTreeMap<String, String>,
}
