use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use serde::Deserialize;

use crate::azure_content_safety::config::AzureContentSafetyConfigModel;
use crate::azure_content_safety::legacy_wire::TextCategory;

use super::wire::OutputType;

// LiteLLM's Azure Text Moderation default, distinct from the legacy analyze hook's four.
pub const DEFAULT_SEVERITY_THRESHOLD: i64 = 2;

// The order LiteLLM's __init__ puts the default categories in, which the request body keeps.
pub const DEFAULT_CATEGORIES: [TextCategory; 4] = [
    TextCategory::Hate,
    TextCategory::Sexual,
    TextCategory::SelfHarm,
    TextCategory::Violence,
];

// LiteLLM's AzureTextModerationOptionalParams.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OptionalParams {
    pub severity_threshold: Option<i64>,
    pub severity_threshold_by_category: Option<BTreeMap<TextCategory, i64>>,
    pub categories: Option<Vec<TextCategory>>,
    #[serde(rename = "blocklistNames")]
    pub blocklist_names: Option<Vec<String>>,
    #[serde(rename = "haltOnBlocklistHit")]
    pub halt_on_blocklist_hit: Option<bool>,
    #[serde(rename = "outputType")]
    pub output_type: Option<OutputType>,
}

// LiteLLM's AzureContentSafetyTextModerationConfigModel, which get_config_model returns.
#[derive(Debug, Clone, Deserialize)]
pub struct AzureTextModerationConfigModel {
    #[serde(flatten)]
    pub base: AzureContentSafetyConfigModel,
    pub optional_params: OptionalParams,
}

impl GuardrailConfigModelIdentity for AzureTextModerationConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Azure Content Safety Text Moderation"
    }
}
