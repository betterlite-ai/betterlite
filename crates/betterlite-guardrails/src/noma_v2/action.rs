use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::generic_guardrail_contracts::GuardrailToolParam;
use crate::guardrail_base::inputs::GuardrailApiInputs;

use super::INTERVENED_INPUT_FIELDS;
use super::error::NomaV2Error;

// LiteLLM's _Action: the only three values a Noma scan answer may carry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Blocked,
    None,
    GuardrailIntervened,
}

#[must_use]
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
    for field in INTERVENED_INPUT_FIELDS {
        let Some(GuardrailValue::Array(items)) = fields.get(field) else {
            continue;
        };
        match field {
            "texts" => inputs.texts = Some(text_items(field, items)?),
            "images" => inputs.images = Some(text_items(field, items)?),
            "tools" => inputs.tools = Some(tool_items(field, items)?),
            "tool_calls" => inputs.tool_calls = Some(items.clone()),
            _other => {}
        }
    }
    Ok(inputs)
}

fn text_items(field: &'static str, items: &[GuardrailValue]) -> Result<Vec<String>, NomaV2Error> {
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        let GuardrailValue::Text(text) = item else {
            return Err(NomaV2Error::Intervention { field });
        };
        texts.push(text.clone());
    }
    Ok(texts)
}

fn tool_items(
    field: &'static str,
    items: &[GuardrailValue],
) -> Result<Vec<GuardrailToolParam>, NomaV2Error> {
    let mut tools = Vec::with_capacity(items.len());
    for item in items {
        let encoded =
            serde_json::to_value(item).map_err(|_not_json| NomaV2Error::Intervention { field })?;
        let tool = serde_json::from_value(encoded)
            .map_err(|_wrong_shape| NomaV2Error::Intervention { field })?;
        tools.push(tool);
    }
    Ok(tools)
}
