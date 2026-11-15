use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::vigil_guard::metadata::CallIdSource;
use crate::vigil_guard::output::{Inputs, ToolCall};

use super::error::VigilRuntimeError;

pub(super) fn from_inputs(
    inputs: &GuardrailApiInputs,
    input_type: GuardrailInputType,
) -> Result<Inputs, VigilRuntimeError> {
    let tool_calls = match input_type {
        GuardrailInputType::Request => None,
        GuardrailInputType::Response => match inputs.tool_calls.as_deref() {
            Some(calls) => Some(scanned_tool_calls(calls)?),
            None => None,
        },
    };
    Ok(Inputs {
        texts: inputs.texts.clone(),
        images: inputs.images.as_ref().map(|images| {
            images
                .iter()
                .map(|image| GuardrailValue::Text(image.clone()))
                .collect()
        }),
        tools: inputs
            .tools
            .as_ref()
            .map(|tools| tools.iter().map(tool_param_value).collect()),
        tool_calls,
        structured_messages: inputs
            .structured_messages
            .as_ref()
            .map(|messages| messages.iter().map(message_value).collect()),
        model: inputs.model.clone(),
        stream_holdback_chars: None,
    })
}

fn scanned_tool_calls(calls: &[GuardrailValue]) -> Result<Vec<ToolCall>, VigilRuntimeError> {
    let mut converted = Vec::with_capacity(calls.len());
    for (index, value) in calls.iter().enumerate() {
        let GuardrailValue::Object(fields) = value else {
            return Err(VigilRuntimeError::InvalidToolCall { index });
        };
        converted.push(ToolCall {
            fields: fields.clone(),
        });
    }
    Ok(converted)
}

// LiteLLM's _build_output: an unchanged result is the caller's own record, a rewrite only keeps the remap keys
pub(super) fn to_output(
    original: &GuardrailApiInputs,
    bridged: &Inputs,
    built: Inputs,
) -> GuardrailApiInputs {
    if &built == bridged {
        return original.clone();
    }
    GuardrailApiInputs {
        texts: built.texts,
        images: original.images.clone(),
        tools: original.tools.clone(),
        tool_calls: built.tool_calls.as_deref().map(tool_call_values),
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn tool_param_value(tool: &GuardrailToolParam) -> GuardrailValue {
    let mut fields = tool.extra.clone();
    fields.insert(
        "type".to_owned(),
        GuardrailValue::Text(tool.tool_type.clone()),
    );
    GuardrailValue::Object(fields)
}

fn message_value(message: &GuardrailMessage) -> GuardrailValue {
    let mut fields = message.extra.clone();
    fields.insert(
        "role".to_owned(),
        GuardrailValue::Text(message.role.clone()),
    );
    GuardrailValue::Object(fields)
}

fn tool_call_values(calls: &[ToolCall]) -> Vec<GuardrailValue> {
    calls
        .iter()
        .map(|call| GuardrailValue::Object(call.fields.clone()))
        .collect()
}

pub(super) struct CallIds<'target> {
    pub(super) target: &'target GuardrailLogTarget,
}

impl CallIdSource for CallIds<'_> {
    fn litellm_call_id(&self) -> Option<&str> {
        [
            self.target.model_call_params.as_ref(),
            self.target.litellm_params.as_ref(),
        ]
        .into_iter()
        .flatten()
        .find_map(call_id)
    }
}

fn call_id(params: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    match params.get("litellm_call_id") {
        Some(GuardrailValue::Text(call_id)) if !call_id.is_empty() => Some(call_id.as_str()),
        _ => None,
    }
}
