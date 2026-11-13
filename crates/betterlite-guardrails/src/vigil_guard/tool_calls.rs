use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::VigilGuardError;
use super::output::ToolCall;

#[must_use]
pub fn tool_call_arguments(tool_calls: Option<&[ToolCall]>) -> Vec<(usize, String)> {
    let mut pairs = Vec::new();
    for (index, tool_call) in tool_calls.unwrap_or_default().iter().enumerate() {
        let Some(GuardrailValue::Object(function)) = tool_call.fields.get("function") else {
            continue;
        };
        let Some(GuardrailValue::Text(arguments)) = function.get("arguments") else {
            continue;
        };
        if arguments.trim().is_empty() {
            continue;
        }
        pairs.push((index, arguments.clone()));
    }
    pairs
}

pub fn set_tool_call_arguments(
    tool_calls: &[ToolCall],
    index: usize,
    arguments: &str,
) -> Result<Vec<ToolCall>, VigilGuardError> {
    let mut updated = tool_calls.to_vec();
    let Some(tool_call) = updated.get_mut(index) else {
        return Err(VigilGuardError::ToolCallIndex {
            index,
            len: tool_calls.len(),
        });
    };
    let mut function = match tool_call.fields.get("function") {
        Some(GuardrailValue::Object(function)) => function.clone(),
        Some(_) => return Err(VigilGuardError::InvalidToolFunction { index }),
        None => BTreeMap::new(),
    };
    function.insert(
        "arguments".to_owned(),
        GuardrailValue::Text(arguments.to_owned()),
    );
    tool_call
        .fields
        .insert("function".to_owned(), GuardrailValue::Object(function));
    Ok(updated)
}
