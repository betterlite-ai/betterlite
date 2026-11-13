use std::collections::BTreeMap;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    MapParams(MapParamsStep),
    Request(RequestStep),
    Reply(ReplyStep),
    Stream(StreamStep),
}

#[derive(Debug, Deserialize)]
pub(crate) struct MapParamsStep {
    pub(crate) params: Object,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestStep {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) api_base: Option<String>,
    pub(crate) env_base: Option<String>,
    pub(crate) token: String,
    #[serde(default)]
    pub(crate) headers: Headers,
    #[serde(default)]
    pub(crate) params: Object,
    #[serde(default)]
    pub(crate) uploads: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplyStep {
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) model: String,
    pub(crate) created: u64,
    #[serde(default)]
    pub(crate) structured_output: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamStep {
    pub(crate) events: Vec<String>,
}
