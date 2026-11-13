use serde::Deserialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::choices::ChoicesError;
use crate::chat_choices::wire::{
    CallFunction, ChoiceToolCall, CustomPayload, FunctionArguments, ReplyToolCall,
};

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
            extra: call.extra.clone(),
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
        extra: call.extra.clone(),
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
        replacements.push((index, expanded_parallel_call(call, parsed)?));
    }
    Ok(splice(calls, replacements))
}

#[derive(Deserialize)]
struct ParallelArguments {
    tool_uses: Vec<ToolUse>,
}

#[derive(Deserialize)]
struct ToolUse {
    recipient_name: String,
    parameters: Value,
}

fn expanded_parallel_call(
    call: &ChoiceToolCall,
    parsed: Value,
) -> Result<Vec<ChoiceToolCall>, ChoicesError> {
    let arguments: ParallelArguments =
        serde_json::from_value(parsed).map_err(|source| ChoicesError::ParallelUses { source })?;
    let call_id = &call.id;
    let mut fixed = Vec::with_capacity(arguments.tool_uses.len());
    for (position, tool_use) in arguments.tool_uses.into_iter().enumerate() {
        let recipient = tool_use.recipient_name;
        let name = recipient.strip_prefix("functions.").unwrap_or(&recipient);
        fixed.push(ChoiceToolCall {
            id: format!("{call_id}_{position}"),
            kind: String::from("function"),
            function: Some(CallFunction {
                name: Some(name.to_owned()),
                arguments: tool_use.parameters.to_string(),
            }),
            custom: None,
            extra: Map::new(),
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

// arguments may be text, an object or null; anything else leaves the content as text
#[derive(Deserialize)]
struct ContentCall {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    #[serde(default)]
    arguments: FunctionArguments,
}

pub(crate) fn content_tool_call(
    content: &str,
    tools: &Value,
    identifier: fn() -> Uuid,
) -> Result<Option<ChoiceToolCall>, ChoicesError> {
    let names = tool_call_names(tools)?;
    let Ok(parsed) = serde_json::from_str::<ContentCall>(content) else {
        return Ok(None);
    };
    if parsed.kind != "function" || !names.contains(&parsed.name) {
        return Ok(None);
    }
    let arguments = parsed.arguments.into_text().unwrap_or_default();
    Ok(Some(ChoiceToolCall {
        id: identifier().to_string(),
        kind: String::from("function"),
        function: Some(CallFunction {
            name: Some(parsed.name),
            arguments,
        }),
        custom: None,
        extra: Map::new(),
    }))
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
