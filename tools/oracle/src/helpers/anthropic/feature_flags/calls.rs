use betterlite_anthropic::feature_flags::FeatureParams;
use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

pub(crate) mod beta;

#[derive(Debug, Deserialize)]
pub(super) struct OneShot {
    pub(super) user_agent: Option<String>,
    pub(super) messages: Vec<Json>,
    pub(super) system: Json,
    pub(super) tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BillingFields {
    pub(super) texts: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Compaction {
    pub(super) provider: String,
    pub(super) params: FeatureParams,
    pub(super) messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CacheControl {
    pub(super) model: String,
    pub(super) custom_llm_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CacheControlSet {
    pub(super) messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FileIds {
    pub(super) messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Pdf {
    pub(super) messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Tools {
    pub(super) tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct EagerFlag {
    pub(super) tool: Json,
}

#[derive(Debug, Deserialize)]
pub(super) struct EagerUsed {
    pub(super) tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Computer {
    pub(super) tools: Option<Vec<Json>>,
    pub(super) versions: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Skills {
    pub(super) tools: Option<Vec<Json>>,
    pub(super) params: FeatureParams,
    pub(super) headers: Vec<Option<String>>,
    pub(super) providers: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Conversation {
    pub(super) messages: Vec<Json>,
    pub(super) thinking: Json,
    pub(super) mcp_servers: Option<Vec<Json>>,
}
