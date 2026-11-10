use std::collections::BTreeMap;

use betterlite_models::available_models::parameters::LiteLlmParams;
use betterlite_models::identity::provider_id::ProviderId;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Enumerate(EnumerateInput),
    Cache(CacheInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnumerateInput {
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub provider_list: Vec<ProviderId>,
    #[serde(default)]
    pub models_by_provider: BTreeMap<ProviderId, Vec<String>>,
    #[serde(default)]
    pub global_check_provider_endpoint: bool,
    pub calls: Vec<CallInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallInput {
    #[serde(default)]
    pub check_provider_endpoint: bool,
    #[serde(default)]
    pub global_check_provider_endpoint: Option<bool>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub litellm_params: LiteLlmParams,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub cache_clock_fails: bool,
    #[serde(default)]
    pub listing: BTreeMap<String, ListingOutcome>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ListingOutcome {
    Models(Vec<String>),
    Error(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheInput {
    pub steps: Vec<CacheStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheStep {
    pub op: CacheOp,
    #[serde(default)]
    pub provider: Option<ProviderId>,
    #[serde(default)]
    pub parameters: Option<LiteLlmParams>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub now: f64,
    #[serde(default)]
    pub clock_fails: bool,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CacheOp {
    Config,
    Get,
    GetMutate,
    Set,
    SetMutate,
}
