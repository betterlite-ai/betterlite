use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct ProviderCase {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct UrlCase {
    pub(super) model: String,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    pub(super) api_base: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ValidateCase {
    pub(super) headers: BTreeMap<String, String>,
    pub(super) api_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct FlowInput {
    pub(super) provider: ProviderCase,
    pub(super) urls: Vec<UrlCase>,
    pub(super) validate: Vec<ValidateCase>,
}

#[derive(Deserialize)]
pub(crate) struct RequestCase {
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
}

#[derive(Deserialize)]
pub(crate) struct SignCase {
    pub(super) headers: BTreeMap<String, String>,
    pub(super) request_data: IndexMap<String, Json>,
}

#[derive(Deserialize)]
pub(crate) struct ReplyCase {
    pub(super) model: String,
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
}
