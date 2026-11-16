use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) targets: Vec<TargetStep>,
    #[serde(default)]
    pub(crate) models: Vec<String>,
    #[serde(default)]
    pub(crate) forms: Vec<FormStep>,
    #[serde(default)]
    pub(crate) urls: Vec<UrlStep>,
    #[serde(default)]
    pub(crate) auth: Vec<AuthStep>,
    #[serde(default)]
    pub(crate) generations: Vec<GenerationStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetStep {
    #[serde(default)]
    pub(crate) azure_endpoint: Option<String>,
    #[serde(default)]
    pub(crate) base_url: Option<String>,
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) base_model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FormStep {
    pub(crate) data: IndexMap<String, Json>,
    pub(crate) request_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) global_base: Option<String>,
    #[serde(default)]
    pub(crate) global_version: Option<String>,
}

// the litellm_params projection the endpoints helper uses: credential keys under auth
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthStep {
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) params: Params,
    #[serde(default)]
    pub(crate) env_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) azure_ad_token: Option<String>,
    #[serde(default)]
    pub(crate) auth: AuthBlock,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthBlock {
    #[serde(default)]
    pub(crate) tenant_id: Option<String>,
    #[serde(default)]
    pub(crate) client_id: Option<String>,
    #[serde(default)]
    pub(crate) client_secret: Option<String>,
    #[serde(default)]
    pub(crate) azure_username: Option<String>,
    #[serde(default)]
    pub(crate) azure_password: Option<String>,
    #[serde(default)]
    pub(crate) azure_scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyStep {
    #[serde(default = "ok_status")]
    pub(crate) status: u16,
    pub(crate) body: String,
}

fn ok_status() -> u16 {
    200
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerationStep {
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) base_model: Option<String>,
    #[serde(default)]
    pub(crate) optional: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_version: Option<String>,
    #[serde(default)]
    pub(crate) azure_ad_token: Option<String>,
    #[serde(default)]
    pub(crate) is_async: bool,
    pub(crate) reply: ReplyStep,
}
