use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use super::convert::{ApiMessage, ApiTool};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualifireResponse {
    pub score: GuardrailValue,
    pub status: GuardrailValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockDetail {
    pub error: String,
    pub qualifire_response: QualifireResponse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvaluationResult {
    #[serde(default)]
    pub results: Option<Vec<FlaggedResult>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FlaggedResult {
    #[serde(default)]
    pub flagged: Option<GuardrailValue>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Verdict {
    #[serde(default, rename = "evaluationResults")]
    pub evaluation_results: Option<Vec<EvaluationResult>>,
    #[serde(default)]
    pub score: Option<GuardrailValue>,
    #[serde(default)]
    pub status: Option<GuardrailValue>,
}

impl Verdict {
    #[must_use]
    pub fn is_flagged(&self) -> bool {
        self.evaluation_results.iter().flatten().any(|evaluation| {
            evaluation
                .results
                .iter()
                .flatten()
                .any(|entry| matches!(entry.flagged.as_ref(), Some(GuardrailValue::Boolean(true))))
        })
    }

    #[must_use]
    pub fn response(&self) -> QualifireResponse {
        QualifireResponse {
            score: self.score.clone().unwrap_or(GuardrailValue::Null),
            status: self.status.clone().unwrap_or(GuardrailValue::Null),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EvaluatePayload<'payload> {
    pub messages: &'payload [ApiMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<&'payload str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_injections: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hallucinations_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pii_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_moderation_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_selection_quality_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_tools: Option<&'payload [ApiTool]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertions: Option<&'payload [String]>,
}

#[derive(Debug, Serialize)]
pub struct InvokePayload<'payload> {
    pub evaluation_id: &'payload str,
    pub input: &'payload str,
    pub output: &'payload str,
    pub messages: &'payload [ApiMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_tools: Option<&'payload [ApiTool]>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Payload<'payload> {
    Invoke(InvokePayload<'payload>),
    Evaluate(EvaluatePayload<'payload>),
}
