use std::collections::BTreeMap;
use std::num::ParseFloatError;

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
#[serde(try_from = "RawGraySwanResponse")]
pub struct GraySwanResponse {
    pub violation: f64,
    pub violated_rule_descriptions: Vec<GraySwanViolatedRule>,
    pub mutation: Option<bool>,
    pub ipi: Option<bool>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawGraySwanResponse {
    violation: Option<ViolationScore>,
    violated_rule_descriptions: Vec<GraySwanViolatedRule>,
    mutation: Option<bool>,
    ipi: Option<bool>,
}
// LiteLLM treats a null, missing or numeric-string violation score the same as a plain float.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ViolationScore {
    PrivateNumber(PrivateNumber),
    Number(f64),
    Text(String),
}
#[derive(Debug, Deserialize)]
struct PrivateNumber {
    #[serde(rename = "$serde_json::private::Number")]
    number: String,
}
#[derive(Debug, Error)]
pub enum GraySwanResponseError {
    #[error("invalid violation score: {text:?}")]
    InvalidViolation {
        text: String,
        #[source]
        source: ParseFloatError,
    },
}
impl TryFrom<RawGraySwanResponse> for GraySwanResponse {
    type Error = GraySwanResponseError;
    fn try_from(raw: RawGraySwanResponse) -> Result<Self, Self::Error> {
        let violation = match raw.violation {
            None => 0.0_f64,
            Some(ViolationScore::PrivateNumber(wrapped)) => {
                wrapped.number.parse::<f64>().map_err(|source| {
                    GraySwanResponseError::InvalidViolation {
                        text: wrapped.number.clone(),
                        source,
                    }
                })?
            }
            Some(ViolationScore::Number(value)) => value,
            Some(ViolationScore::Text(text)) if text.is_empty() => 0.0_f64,
            Some(ViolationScore::Text(text)) => text.trim().parse::<f64>().map_err(|source| {
                GraySwanResponseError::InvalidViolation {
                    text: text.clone(),
                    source,
                }
            })?,
        };
        Ok(Self {
            violation,
            violated_rule_descriptions: raw.violated_rule_descriptions,
            mutation: raw.mutation,
            ipi: raw.ipi,
        })
    }
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
impl GraySwanViolatedRuleFields {
    #[must_use]
    pub fn to_fields(&self) -> BTreeMap<String, GuardrailExtraValue> {
        let mut map = self.extra.clone();
        if let Some(number) = &self.rule {
            map.insert("rule".to_owned(), number.to_extra_value());
        }
        if let Some(name) = &self.name {
            map.insert("name".to_owned(), GuardrailExtraValue::Text(name.clone()));
        }
        if let Some(description) = &self.description {
            map.insert(
                "description".to_owned(),
                GuardrailExtraValue::Text(description.clone()),
            );
        }
        map
    }
}
impl GraySwanViolatedRule {
    #[must_use]
    pub fn to_extra_value(&self) -> GuardrailExtraValue {
        match self {
            Self::Structured(fields) => GuardrailExtraValue::Object(fields.to_fields()),
            Self::Plain(value) => value.clone(),
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GraySwanRuleNumber {
    Integer(i64),
    Text(String),
}
impl GraySwanRuleNumber {
    #[must_use]
    pub fn to_extra_value(&self) -> GuardrailExtraValue {
        match self {
            Self::Integer(value) => GuardrailExtraValue::SignedInteger(*value),
            Self::Text(value) => GuardrailExtraValue::Text(value.clone()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraySwanVerdict {
    Pass,
    Monitored { violation_message: String },
    Replaced { violation_message: String },
    ShortCircuited { violation_message: String },
}
#[derive(Debug, Clone)]
pub struct GraySwanResult {
    pub verdict: GraySwanVerdict,
    pub messages: Vec<GuardrailMessage>,
}
