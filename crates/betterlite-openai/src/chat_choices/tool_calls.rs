use serde_json::Value;
use uuid::Uuid;

use super::choices::ChoicesError;
use crate::chat_choices::wire::{CallFunction, ChoiceToolCall, CustomPayload, ReplyToolCall};

const PARALLEL_TOOL: &str = "multi_tool_use.parallel";

pub(crate) fn convert_tool_calls(
    calls: &[ReplyToolCall],
    identifier: fn() -> Uuid,
) -> Result<Vec<ChoiceToolCall>, ChoicesError> {
    calls
        .iter()
        .map(|call| convert_tool_call(call, identifier))
        .collect()
}

fn convert_tool_call(
    call: &ReplyToolCall,
    identifier: fn() -> Uuid,
) -> Result<ChoiceToolCall, ChoicesError> {
    let custom = call.custom.as_ref();
    if custom.is_some() || call.kind.as_deref() == Some("custom") {
        let payload = custom.ok_or(ChoicesError::MissingCustom)?;
        return Ok(ChoiceToolCall {
            id: call.id.clone().ok_or(ChoicesError::MissingCustomId)?,
            kind: String::from("custom"),
            function: None,
            custom: Some(CustomPayload {
                name: payload.name.clone(),
                input: payload.input.clone(),
            }),
        });
    }
    let function = call
        .function
        .as_ref()
        .ok_or(ChoicesError::MissingFunction)?;
    let id = call.id.clone().unwrap_or_else(|| identifier().to_string());
    Ok(ChoiceToolCall {
        id,
        kind: String::from("function"),
        function: Some(CallFunction {
            name: function.name.clone(),
            arguments: function.arguments.clone().unwrap_or_default(),
        }),
        custom: None,
    })
}

pub(crate) fn repair_parallel_tool_calls(
    calls: Vec<ChoiceToolCall>,
) -> Result<Vec<ChoiceToolCall>, ChoicesError> {
    let original = calls.clone();
    let mut replacements: Vec<(usize, Vec<ChoiceToolCall>)> = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let Some(function) = call.function.as_ref() else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(&function.arguments) else {
            return Ok(original);
        };
        if function.name.as_deref() != Some(PARALLEL_TOOL) {
            continue;
        }
        replacements.push((index, expanded_parallel_call(call, &parsed)?));
    }
    Ok(splice(calls, replacements))
}

fn expanded_parallel_call(
    call: &ChoiceToolCall,
    parsed: &Value,
) -> Result<Vec<ChoiceToolCall>, ChoicesError> {
    let uses = parsed
        .get("tool_uses")
        .and_then(Value::as_array)
        .ok_or(ChoicesError::ParallelUses)?;
    let mut fixed = Vec::with_capacity(uses.len());
    for (position, tool_use) in uses.iter().enumerate() {
        let recipient = tool_use
            .get("recipient_name")
            .and_then(Value::as_str)
            .ok_or(ChoicesError::ParallelEntry)?;
        let parameters = tool_use
            .get("parameters")
            .ok_or(ChoicesError::ParallelEntry)?;
        let name = recipient.strip_prefix("functions.").unwrap_or(recipient);
        let call_id = &call.id;
        fixed.push(ChoiceToolCall {
            id: format!("{call_id}_{position}"),
            kind: String::from("function"),
            function: Some(CallFunction {
                name: Some(name.to_owned()),
                arguments: parameters.to_string(),
            }),
            custom: None,
        });
    }
    Ok(fixed)
}

fn splice(
    calls: Vec<ChoiceToolCall>,
    replaced: Vec<(usize, Vec<ChoiceToolCall>)>,
) -> Vec<ChoiceToolCall> {
    if replaced.is_empty() {
        return calls;
    }
    let mut result = Vec::with_capacity(calls.len());
    let mut replacements = replaced.into_iter().peekable();
    for (index, call) in calls.into_iter().enumerate() {
        if replacements
            .peek()
            .is_some_and(|(position, _)| *position == index)
        {
            if let Some((_, fixed)) = replacements.next() {
                result.extend(fixed);
            }
            continue;
        }
        result.push(call);
    }
    result.extend(replacements.flat_map(|(_, fixed)| fixed));
    result
}

pub(crate) fn content_tool_call(
    content: &str,
    tools: &Value,
    identifier: fn() -> Uuid,
) -> Result<Option<ChoiceToolCall>, ChoicesError> {
    let names = tool_call_names(tools)?;
    let Ok(parsed) = serde_json::from_str::<Value>(content) else {
        return Ok(None);
    };
    let Some(fields) = parsed.as_object() else {
        return Ok(None);
    };
    if fields.get("type").and_then(Value::as_str) != Some("function") {
        return Ok(None);
    }
    let Some(name) = fields.get("name").and_then(Value::as_str) else {
        return Ok(None);
    };
    if !names.iter().any(|known| known == name) {
        return Ok(None);
    }
    let Some(arguments) = arguments_text(fields.get("arguments")) else {
        return Ok(None);
    };
    Ok(Some(ChoiceToolCall {
        id: identifier().to_string(),
        kind: String::from("function"),
        function: Some(CallFunction {
            name: Some(name.to_owned()),
            arguments,
        }),
        custom: None,
    }))
}

fn arguments_text(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => Some(String::new()),
        Some(Value::String(text)) => Some(text.clone()),
        Some(object @ Value::Object(_)) => Some(object.to_string()),
        Some(Value::Array(_) | Value::Bool(_) | Value::Number(_)) => None,
    }
}

fn tool_call_names(tools: &Value) -> Result<Vec<String>, ChoicesError> {
    let entries = tools.as_array().ok_or(ChoicesError::ToolsNotList)?;
    let mut names = Vec::new();
    for tool in entries {
        let fields = tool.as_object().ok_or(ChoicesError::ToolNotObject)?;
        if fields.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        match fields.get("function") {
            None => {}
            Some(Value::Object(function)) => {
                if let Some(found) = function
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|given| !given.is_empty())
                {
                    names.push(found.to_owned());
                }
            }
            Some(_) => return Err(ChoicesError::ToolNotObject),
        }
    }
    Ok(names)
}
