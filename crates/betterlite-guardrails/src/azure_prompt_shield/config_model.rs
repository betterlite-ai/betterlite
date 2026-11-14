use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::azure_content_safety::config::AzureContentSafetyConfigModel;

// LiteLLM's AzurePromptShieldGuardrailConfigModel adds the billing fields to the content safety ones.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "RawConfigModel")]
pub struct AzurePromptShieldConfigModel {
    pub content_safety: AzureContentSafetyConfigModel,
    pub cost_tier: Option<String>,
    pub price_per_1000_text_records: Option<Decimal>,
}

#[derive(Deserialize)]
struct RawConfigModel {
    #[serde(flatten)]
    content_safety: AzureContentSafetyConfigModel,
    cost_tier: Option<String>,
    price_per_1000_text_records: Option<Decimal>,
}

impl From<RawConfigModel> for AzurePromptShieldConfigModel {
    fn from(raw: RawConfigModel) -> Self {
        Self {
            content_safety: raw.content_safety,
            cost_tier: raw.cost_tier,
            price_per_1000_text_records: raw.price_per_1000_text_records,
        }
    }
}

impl Default for AzurePromptShieldConfigModel {
    fn default() -> Self {
        Self::from(RawConfigModel {
            content_safety: AzureContentSafetyConfigModel::default(),
            cost_tier: None,
            price_per_1000_text_records: None,
        })
    }
}

impl GuardrailConfigModelIdentity for AzurePromptShieldConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Azure Content Safety Prompt Shield"
    }
}
