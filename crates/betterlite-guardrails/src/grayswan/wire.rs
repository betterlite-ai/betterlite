use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailMessage, GuardrailToolParam,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraySwanInput {
    pub messages: Vec<GuardrailMessage>,
    #[serde(default)]
    pub tools: Vec<GuardrailToolParam>,
    #[serde(default)]
    pub metadata: BTreeMap<String, GuardrailExtraValue>,
    #[serde(default)]
    pub request_history: Vec<GuardrailMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraySwanPayload {
    pub messages: Vec<GuardrailMessage>,
    pub tools: Vec<GuardrailToolParam>,
    pub parameters: BTreeMap<String, GuardrailExtraValue>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraySwanResponse {
    #[serde(default)]
    pub violation_score: f64,
    #[serde(default)]
    pub violation_reason: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<GuardrailMessage>>,
}

#[derive(Debug, Clone)]
pub struct GraySwanResult {
    pub flagged: bool,
    pub messages: Vec<GuardrailMessage>,
    pub violation_reason: Option<String>,
}
