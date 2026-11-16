use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCase {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
}

// The case file also carries a model for the LiteLLM side, which validate_environment does not read
#[derive(Deserialize)]
pub(crate) struct HeadersCase {
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) optional_params: Object,
    pub(super) user_agent: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamsCase {
    pub(super) model: String,
    pub(super) non_default_params: Object,
    pub(super) optional_params: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesCase {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCase {
    pub(super) body: Json,
    pub(super) json_mode: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamCase {
    pub(super) chunks: Vec<Json>,
    pub(super) json_mode: bool,
}
