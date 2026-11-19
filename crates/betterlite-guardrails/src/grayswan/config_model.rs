use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

// LiteLLM's GraySwanGuardrailConfigModelOptionalParams, whose pydantic Field defaults a missing key keeps
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraySwanConfigOptionalParams {
    #[serde(default = "default_on_flagged_action")]
    pub on_flagged_action: Option<String>,
    #[serde(default = "default_violation_threshold")]
    pub violation_threshold: Option<f64>,
    #[serde(default)]
    pub reasoning_mode: Option<String>,
    #[serde(default)]
    pub policy_id: Option<String>,
    #[serde(default)]
    pub categories: Option<BTreeMap<String, String>>,
    #[serde(default = "default_fail_open")]
    pub fail_open: Option<bool>,
    #[serde(default = "default_guardrail_timeout")]
    pub guardrail_timeout: Option<f64>,
}

// A key held as a SecretString cannot be serialized back out without exposing it, and nothing writes this model
#[derive(Debug, Clone, Deserialize)]
pub struct GraySwanGuardrailConfigModel {
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub optional_params: Option<GraySwanConfigOptionalParams>,
}

impl GuardrailConfigModelIdentity for GraySwanGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Gray Swan Guardrail"
    }
}

#[must_use]
pub fn default_on_flagged_action() -> Option<String> {
    Some("passthrough".to_owned())
}

#[must_use]
pub const fn default_violation_threshold() -> Option<f64> {
    Some(0.5)
}

#[must_use]
pub const fn default_fail_open() -> Option<bool> {
    Some(true)
}

#[must_use]
pub const fn default_guardrail_timeout() -> Option<f64> {
    Some(30.0)
}
