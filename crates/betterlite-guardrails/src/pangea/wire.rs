use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::GuardrailMessage;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiGuardRequest {
    pub(crate) debug: bool,
    pub(crate) input: AiGuardInput,
    pub(crate) event_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recipe: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiGuardInput {
    pub(crate) messages: Option<Vec<GuardrailMessage>>,
    pub(crate) tools: Option<GuardrailValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) choices: Option<Vec<GuardrailValue>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct AiGuardResponse {
    pub(crate) result: AiGuardResult,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AiGuardResult {
    pub(crate) blocked: Option<bool>,
    pub(crate) transformed: Option<bool>,
    pub(crate) output: Option<AiGuardOutput>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AiGuardOutput {
    pub(crate) messages: Option<Vec<GuardrailMessage>>,
    pub(crate) choices: Option<Vec<GuardrailValue>>,
}
