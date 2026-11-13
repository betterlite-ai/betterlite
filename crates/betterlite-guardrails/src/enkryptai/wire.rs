use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectPayload {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectionResponse {
    pub summary: Summary,
    pub details: Details,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toxicity: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_violation: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pii: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword_detected: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_injection: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jailbreak: Option<i64>,
    #[serde(flatten)]
    pub other: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_violation: Option<PolicyViolationDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pii: Option<PiiDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toxicity: Option<ToxicityDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword_detected: Option<KeywordDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias: Option<BiasDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_injection: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jailbreak: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(flatten)]
    pub other: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyViolationDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violating_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PiiDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pii: Option<BTreeMap<String, GuardrailValue>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToxicityDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toxic: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severe_toxic: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obscene: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insult: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_hate: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeywordDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_keywords: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BiasDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias_detected: Option<bool>,
}
