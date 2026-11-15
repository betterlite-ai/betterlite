use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use secrecy::SecretString;
use serde::Deserialize;

mod tests;

pub const AGENT_365_PROD_API_BASE: &str = "https://agent365.svc.cloud.microsoft";
pub const AGENT_365_PROD_RESOURCE_APP_ID: &str = "ea9ffc3e-8a23-4a7d-836d-234d7c7565c1";
pub const AGENT_365_SCOPE_NAME: &str = "ThreatProtection.Evaluate.All";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent365GuardrailConfigModel {
    pub optional_params: Option<GuardrailValue>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub unreachable_fallback: UnreachableFallback,
}

impl GuardrailConfigModelIdentity for Agent365GuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Microsoft Agent 365"
    }
}
