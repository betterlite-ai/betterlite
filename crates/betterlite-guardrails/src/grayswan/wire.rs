use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailMessage, GuardrailToolParam,
};
use crate::grayswan::ReasoningMode;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GraySwanDynamicParams {
    pub categories: Option<BTreeMap<String, String>>,
    pub policy_id: Option<String>,
    pub reasoning_mode: Option<ReasoningMode>,
    pub metadata: Option<GuardrailExtraValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraySwanInput {
    #[serde(default)]
    pub texts: Vec<String>,
    #[serde(default)]
    pub tool_calls: Vec<GuardrailExtraValue>,
    #[serde(default)]
    pub tools: Vec<GuardrailToolParam>,
    #[serde(default)]
    pub metadata: GraySwanDynamicParams,
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
    pub violated_rule_descriptions: Vec<GraySwanViolatedRule>,
    #[serde(default)]
    pub mutation: Option<bool>,
    #[serde(default)]
    pub ipi: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GraySwanViolatedRule {
    Structured(GraySwanViolatedRuleFields),
    Plain(GuardrailExtraValue),
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraySwanViolatedRuleFields {
    #[serde(default)]
    pub rule: Option<GraySwanRuleNumber>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailExtraValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GraySwanRuleNumber {
    Integer(i64),
    Text(String),
}

impl GraySwanRuleNumber {
    #[must_use]
    pub const fn is_truthy(&self) -> bool {
        match self {
            Self::Integer(value) => *value != 0,
            Self::Text(value) => !value.is_empty(),
        }
    }
}

impl fmt::Display for GraySwanRuleNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(f, "{value}"),
            Self::Text(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GraySwanResult {
    pub flagged: bool,
    pub messages: Vec<GuardrailMessage>,
    pub violation_message: Option<String>,
}
