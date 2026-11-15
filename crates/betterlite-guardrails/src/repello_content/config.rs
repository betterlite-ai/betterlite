use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;
use serde::Deserialize;

use crate::generic_guardrail_contracts::UnreachableFallback;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepelloAIGuardrailConfigModel {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub asset_id: Option<String>,
    #[serde(default = "default_unreachable_fallback")]
    pub unreachable_fallback: UnreachableFallback,
    pub optional_params: Option<BTreeMap<String, GuardrailValue>>,
}

const fn default_unreachable_fallback() -> UnreachableFallback {
    UnreachableFallback::FailClosed
}

impl GuardrailConfigModelIdentity for RepelloAIGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "RepelloAI Argus"
    }
}
