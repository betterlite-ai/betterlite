use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailToolParam;
use crate::guardrail_base::inputs::GuardrailApiInputs;

use super::error::NomaV2Error;

// LiteLLM's _Action: the only three values a Noma scan answer may carry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Blocked,
    None,
    GuardrailIntervened,
}

pub fn parse(response: &GuardrailValue) -> Result<Action, NomaV2Error> {
    let GuardrailValue::Object(fields) = response else {
        return Err(NomaV2Error::InvalidAction);
    };
    let Some(GuardrailValue::Text(action)) = fields.get("action") else {
        return Err(NomaV2Error::InvalidAction);
    };
    match action.as_str() {
        "BLOCKED" => Ok(Action::Blocked),
        "NONE" => Ok(Action::None),
        "GUARDRAIL_INTERVENED" => Ok(Action::GuardrailIntervened),
        _other => Err(NomaV2Error::InvalidAction),
    }
}

// LiteLLM's _apply_action for GUARDRAIL_INTERVENED: a list-valued field replaces the inputs field
pub fn intervene(
    mut inputs: GuardrailApiInputs,
    response: &GuardrailValue,
) -> Result<GuardrailApiInputs, NomaV2Error> {
    let GuardrailValue::Object(fields) = response else {
        return Ok(inputs);
    };
    if let Some(GuardrailValue::Array(items)) = fields.get("texts") {
        inputs.texts = Some(converted("texts", items, text_value)?);
    }
    if let Some(GuardrailValue::Array(items)) = fields.get("images") {
        inputs.images = Some(converted("images", items, text_value)?);
    }
    if let Some(GuardrailValue::Array(items)) = fields.get("tools") {
        inputs.tools = Some(converted("tools", items, tool_value)?);
    }
    if let Some(GuardrailValue::Array(items)) = fields.get("tool_calls") {
        inputs.tool_calls = Some(items.clone());
    }
    Ok(inputs)
}

fn converted<T>(
    field: &'static str,
    items: &[GuardrailValue],
    convert: fn(&GuardrailValue) -> Option<T>,
) -> Result<Vec<T>, NomaV2Error> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Some(value) = convert(item) else {
            return Err(NomaV2Error::Intervention { field });
        };
        values.push(value);
    }
    Ok(values)
}

fn text_value(item: &GuardrailValue) -> Option<String> {
    if let GuardrailValue::Text(text) = item {
        return Some(text.clone());
    }
    None
}

fn tool_value(item: &GuardrailValue) -> Option<GuardrailToolParam> {
    let encoded = serde_json::to_value(item).ok()?;
    serde_json::from_value(encoded).ok()
}
