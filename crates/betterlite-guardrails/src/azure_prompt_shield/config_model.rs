use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::azure_content_safety::config::AzureContentSafetyConfigModel;

// LiteLLM's AzurePromptShieldGuardrailConfigModel adds the billing fields to the content safety ones.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AzurePromptShieldConfigModel {
    #[serde(flatten)]
    pub content_safety: AzureContentSafetyConfigModel,
    pub cost_tier: Option<String>,
    pub price_per_1000_text_records: Option<Decimal>,
}

impl GuardrailConfigModelIdentity for AzurePromptShieldConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Azure Content Safety Prompt Shield"
    }
}
