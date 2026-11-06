use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::vigil_guard_handler_protocol::RequestTimeout;

#[derive(Debug, Clone)]
pub struct VigilGuardSettings {
    pub api_base: String,
    pub api_key: SecretString,
    pub timeout: RequestTimeout,
}

// The wire model writes the secret out for the admin UI, so it derives no Debug and resolves to SecretString before use.
#[derive(Clone, Serialize, Deserialize)]
pub struct VigilGuardGuardrailConfigModel {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
}

impl GuardrailConfigModelIdentity for VigilGuardGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Vigil Guard"
    }
}
