use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptShieldRequest<'prompt> {
    pub user_prompt: &'prompt str,
    pub documents: &'prompt [String],
}

// UserPromptAnalysis is total=False, so attackDetected may be absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPromptAnalysis {
    #[serde(
        default,
        rename = "attackDetected",
        skip_serializing_if = "Option::is_none"
    )]
    pub attack_detected: Option<bool>,
}

impl UserPromptAnalysis {
    // LiteLLM interpolates the analysis dict into the detection message; standard JSON stands in for its repr.
    #[must_use]
    pub fn detection_message(&self) -> String {
        let analysis = serde_json::to_string(self).unwrap_or_else(|_source| "{}".to_owned());
        format!("Attack detected: {analysis}")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptShieldResponse {
    #[serde(rename = "userPromptAnalysis")]
    pub user_prompt_analysis: UserPromptAnalysis,
    #[serde(default, rename = "documentsAnalysis")]
    pub documents_analysis: Vec<BTreeMap<String, GuardrailValue>>,
}
