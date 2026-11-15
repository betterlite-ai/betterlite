use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::GuardrailInputType;

pub const VERDICT_ALLOW: &str = "ALLOW";
pub const VERDICT_BLOCK: &str = "BLOCK";
pub const VERDICT_MASK: &str = "MASK";
pub const VERDICT_DETECT: &str = "DETECT";

// index is positional against the submitted texts and a bad one refuses the whole verdict
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AliceReplacement {
    pub index: Option<i64>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AliceVerdict {
    pub verdict: Option<String>,
    pub categories: Option<Vec<String>>,
    pub correlation_id: Option<String>,
    pub message: Option<String>,
    pub replacements: Option<Vec<AliceReplacement>>,
}

#[derive(Debug, Serialize)]
pub struct EvaluateRequest {
    pub input_type: GuardrailInputType,
    pub inputs: GuardrailValue,
    pub request_data: GuardrailValue,
}
