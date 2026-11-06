use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use serde::{Deserialize, Serialize};

// the admin UI writes the tracker key back out, so this wire model derives no Debug and resolves to SecretString at the edge
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OvalixGuardrailConfigModel {
    pub tracker_api_base: Option<String>,
    pub tracker_api_key: Option<String>,
    pub application_id: Option<String>,
    pub pre_checkpoint_id: Option<String>,
    pub post_checkpoint_id: Option<String>,
}

impl GuardrailConfigModelIdentity for OvalixGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Ovalix Guardrail"
    }
}

mod tests;
