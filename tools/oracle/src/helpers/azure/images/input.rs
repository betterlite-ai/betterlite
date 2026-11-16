use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Input {
    pub(crate) targets: Vec<Step>,
    pub(crate) models: Vec<String>,
    pub(crate) forms: Vec<Step>,
    pub(crate) urls: Vec<Step>,
    pub(crate) auth: Vec<Step>,
    pub(crate) generations: Vec<Step>,
}

// one step shape covers every section, and each section's reader picks the keys it means
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Step {
    pub(crate) azure_endpoint: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) base_model: Option<String>,
    pub(crate) data: IndexMap<String, Json>,
    pub(crate) request_url: String,
    pub(crate) global_base: Option<String>,
    pub(crate) global_version: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_key: Option<String>,
    pub(crate) params: Params,
    pub(crate) env_token: Option<String>,
    pub(crate) prompt: String,
    pub(crate) optional: IndexMap<String, Json>,
    pub(crate) azure_ad_token: Option<String>,
    pub(crate) reply_body: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Params {
    pub(crate) api_key: Option<String>,
    pub(crate) azure_ad_token: Option<String>,
}
