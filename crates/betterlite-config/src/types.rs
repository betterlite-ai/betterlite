use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub model_list: Vec<ModelDeployment>,
    pub general_settings: GeneralSettings,
    pub router_settings: RouterSettings,
    pub litellm_settings: LiteLlmSettings,
    pub environment_variables: BTreeMap<String, Option<SecretString>>,
}

#[derive(Debug, Clone)]
pub struct ModelDeployment {
    pub model_name: String,
    pub litellm_params: LiteLlmParams,
    pub model_info: ModelInfo,
}

#[derive(Debug, Clone)]
pub struct LiteLlmParams {
    pub model: Option<String>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub timeout: Option<f64>,
    pub stream_timeout: Option<f64>,
    pub max_retries: Option<u32>,
    pub rpm: Option<u32>,
    pub tpm: Option<u32>,
    pub weight: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelInfo {
    pub id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GeneralSettings {
    pub master_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct RouterSettings {
    pub routing_strategy: Option<RoutingStrategy>,
    pub num_retries: Option<u32>,
    pub timeout: Option<f64>,
    pub allowed_fails: Option<u32>,
    pub cooldown_time: Option<f64>,
    pub fallbacks: Vec<RouterFallback>,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingStrategy {
    SimpleShuffle,
    LeastBusy,
    UsageBasedRouting,
    LatencyBasedRouting,
    CostBasedRouting,
    UsageBasedRoutingV2,
    Lar1,
}
pub type RouterFallback = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone)]
pub struct LiteLlmSettings {
    pub num_retries: Option<u32>,
    pub request_timeout: Option<f64>,
    pub drop_params: Option<bool>,
    pub bedrock_metadata_fields: Vec<String>,
    pub modify_params: bool,
    pub default_max_tokens: i64,
}
