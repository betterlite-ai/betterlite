use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailMessage, GuardrailToolParam,
};

// LiteLLM's GenericGuardrailAPIInputs
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardrailApiInputs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GuardrailToolParam>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<GuardrailExtraValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_messages: Option<Vec<GuardrailMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_holdback_chars: Option<Vec<i64>>,
}

// The two dicts of LiteLLM's logging object that guardrail records are written to
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuardrailLogTarget {
    pub litellm_params: Option<BTreeMap<String, GuardrailValue>>,
    pub model_call_params: Option<BTreeMap<String, GuardrailValue>>,
}
