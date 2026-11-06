use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCall {
    pub fields: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_messages: Option<Vec<GuardrailValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_holdback_chars: Option<Vec<u32>>,
}

#[must_use]
pub fn build_output(
    inputs: &Inputs,
    final_texts: &[String],
    final_tool_calls: Option<&[ToolCall]>,
) -> Inputs {
    let texts_changed = final_texts != inputs.texts.as_deref().unwrap_or_default();
    let tool_calls_changed = final_tool_calls != inputs.tool_calls.as_deref();
    if !texts_changed && !tool_calls_changed {
        return inputs.clone();
    }
    Inputs {
        texts: Some(final_texts.to_vec()),
        images: inputs.images.clone(),
        tools: inputs.tools.clone(),
        tool_calls: if tool_calls_changed {
            final_tool_calls.map(<[ToolCall]>::to_vec)
        } else {
            None
        },
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}
