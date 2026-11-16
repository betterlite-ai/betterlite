use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Platform {
    A2a,
    Langgraph,
    Langflow,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentCase {
    pub(super) agent_name: String,
    pub(super) agent_card_params: Object,
    pub(super) litellm_params: Object,
}

#[derive(Debug, Deserialize)]
pub(super) struct GlobalsCase {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct UpstreamCase {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct DispatchCase {
    pub(super) platform: Platform,
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) stream: Option<bool>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) optionals: Object,
    pub(super) globals: GlobalsCase,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) registry: Vec<AgentCase>,
    pub(super) upstream: UpstreamCase,
}
