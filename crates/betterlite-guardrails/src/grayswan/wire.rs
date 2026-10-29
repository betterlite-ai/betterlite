use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailMessage, GuardrailToolParam,
};
use crate::grayswan::ReasoningMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraySwanInput {
    pub messages: Vec<GuardrailMessage>,
    #[serde(default)]
    pub tools: Vec<GuardrailToolParam>,
    #[serde(default)]
    pub metadata: BTreeMap<String, GuardrailExtraValue>,
    #[serde(default)]
    pub request_history: Vec<GuardrailMessage>,
    #[serde(default)]
    pub litellm_metadata: BTreeMap<String, GuardrailExtraValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraySwanPayload {
    pub messages: Vec<GuardrailMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<GuardrailToolParam>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_mode: Option<ReasoningMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<GuardrailExtraValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub litellm_metadata: Option<GuardrailExtraValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GraySwanResponse {
    #[serde(default)]
    pub violation: f64,
    #[serde(default)]
    pub violated_rule_descriptions: Vec<GuardrailExtraValue>,
    #[serde(default)]
    pub mutation: Option<bool>,
    #[serde(default)]
    pub ipi: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct GraySwanResult {
    pub flagged: bool,
    pub messages: Vec<GuardrailMessage>,
    pub violation_message: Option<String>,
}
