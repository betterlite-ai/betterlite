use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DetectPayload {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DetectionResponse {
    pub summary: Summary,
    pub details: Details,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub toxicity: Option<Vec<String>>,
    pub policy_violation: Option<i64>,
    pub pii: Option<i64>,
    pub keyword_detected: Option<i64>,
    pub bias: Option<i64>,
    pub prompt_injection: Option<i64>,
    pub jailbreak: Option<i64>,
    #[serde(flatten)]
    pub other: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Details {
    pub policy_violation: Option<PolicyViolationDetail>,
    pub pii: Option<PiiDetail>,
    pub toxicity: Option<ToxicityDetail>,
    pub keyword_detected: Option<KeywordDetail>,
    pub bias: Option<BiasDetail>,
    pub prompt_injection: Option<BTreeMap<String, GuardrailValue>>,
    pub jailbreak: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(flatten)]
    pub other: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PolicyViolationDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violating_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PiiDetail {
    pub pii: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct KeywordDetail {
    pub detected_keywords: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BiasDetail {
    pub bias_detected: bool,
}
