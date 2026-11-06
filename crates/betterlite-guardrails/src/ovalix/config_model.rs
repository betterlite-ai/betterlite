use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::GuardrailExtraValue;

// the admin UI writes the tracker key back out, so this wire model derives no Debug and resolves to SecretString at the edge
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OvalixGuardrailConfigModel {
    pub optional_params: Option<BTreeMap<String, GuardrailExtraValue>>,
    pub tracker_api_base: Option<String>,
    pub tracker_api_key: Option<String>,
    pub application_id: Option<String>,
    pub pre_checkpoint_id: Option<String>,
    pub post_checkpoint_id: Option<String>,
}

impl OvalixGuardrailConfigModel {
    pub const TRACKER_API_BASE_DESCRIPTION: &str = "Base URL for the Ovalix Tracker service.";
    pub const TRACKER_API_KEY_DESCRIPTION: &str = "API key for the Ovalix Tracker service.";
    pub const APPLICATION_ID_DESCRIPTION: &str = "Application ID for the Ovalix Tracker service.";
    pub const PRE_CHECKPOINT_ID_DESCRIPTION: &str =
        "Pre-checkpoint ID for the Ovalix Tracker service.";
    pub const POST_CHECKPOINT_ID_DESCRIPTION: &str =
        "Post-checkpoint ID for the Ovalix Tracker service.";
}

impl GuardrailConfigModelIdentity for OvalixGuardrailConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Ovalix Guardrail"
    }
}

mod tests;
