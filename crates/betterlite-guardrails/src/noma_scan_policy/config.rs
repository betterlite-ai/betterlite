use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use secrecy::SecretString;
use serde::Deserialize;

// LiteLLM declares use_v2 with Field(default=False), so an absent key is False and an explicit null keeps None
#[must_use]
pub const fn default_use_v2() -> Option<bool> {
    Some(false)
}

#[derive(Clone, Deserialize)]
pub struct NomaGuardrailConfigModel {
    #[serde(default = "default_use_v2")]
    pub use_v2: Option<bool>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub application_id: Option<String>,
}

impl GuardrailConfigModelIdentity for NomaGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Noma Security"
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct NomaV2GuardrailConfigModel {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub application_id: Option<String>,
    pub gateway_name: Option<String>,
    pub monitor_mode: Option<bool>,
    pub block_failures: Option<bool>,
}

impl GuardrailConfigModelIdentity for NomaV2GuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Noma Security v2"
    }
}
