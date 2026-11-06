use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::ModerationModel;

#[derive(Debug, Serialize)]
pub struct ModerationRequest<'text> {
    pub model: ModerationModel,
    pub input: &'text str,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModerationResult {
    #[serde(default)]
    pub categories: Option<IndexMap<String, bool>>,
    #[serde(default)]
    pub category_applied_input_types: Option<IndexMap<String, GuardrailValue>>,
    #[serde(default)]
    pub category_scores: Option<IndexMap<String, Number>>,
    #[serde(default)]
    pub flagged: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModerationResponse {
    pub id: String,
    pub model: String,
    pub results: Vec<ModerationResult>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViolationDetails {
    pub violated_categories: Vec<String>,
    pub category_scores: IndexMap<String, Number>,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail<'error> {
    pub error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<&'error str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation_result: Option<&'error ViolationDetails>,
}
