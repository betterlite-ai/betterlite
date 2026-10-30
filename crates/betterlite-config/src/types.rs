use std::collections::BTreeMap;

use betterlite_secrets::secret_codec::CipherSettings;
use secrecy::SecretString;
use serde::Deserialize;

use crate::sections::Sections;

#[derive(Debug, Clone)]
pub struct Config {
    pub model_list: Vec<ModelDeployment>,
    pub general_settings: GeneralSettings,
    pub router_settings: RouterSettings,
    pub litellm_settings: LiteLlmSettings,
    pub environment_variables: BTreeMap<String, Option<SecretString>>,
    pub sections: Sections,
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

#[derive(Debug, Clone)]
pub struct GeneralSettings {
    pub cipher: CipherSettings,
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

#[derive(Debug, Clone, Copy, Default)]
pub struct ProxyTrust {
    pub aiohttp_trust_env: bool,
    pub disable_aiohttp_trust_env: bool,
}

#[derive(Clone)]
pub struct LiteLlmSettings {
    pub num_retries: Option<u32>,
    pub request_timeout: Option<f64>,
    pub drop_params: Option<bool>,
    pub proxy_trust: ProxyTrust,
    pub bedrock_metadata_fields: Vec<String>,
    pub modify_params: bool,
    pub default_max_tokens: i64,
    pub cost_discount_config: CostAdjustmentMap,
    pub cost_margin_config: CostAdjustmentMap,
}

impl std::fmt::Debug for LiteLlmSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiteLlmSettings")
            .field("num_retries", &self.num_retries)
            .field("request_timeout", &self.request_timeout)
            .field("drop_params", &self.drop_params)
            .field("proxy_trust", &self.proxy_trust)
            .field("bedrock_metadata_fields", &self.bedrock_metadata_fields)
            .field("modify_params", &self.modify_params)
            .field("default_max_tokens", &self.default_max_tokens)
            .finish()
    }
}

pub type CostAdjustmentMap = BTreeMap<String, serde_json::Value>;
