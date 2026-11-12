use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::chat::choice_finish_reason;
use crate::chat_choices::wire::{
    CallFunction, ChoiceMessage, ChoiceToolCall, CustomPayload, ReplyChoice, ReplyMessage,
    ReplyToolCall, TransformedChoice,
};
use crate::openai_base_config::Object;
use crate::text_completion_policy::reply::mapped_finish_reason;

const JSON_MODE_TOOL: &str = "json_tool_call";
const PARALLEL_TOOL: &str = "multi_tool_use.parallel";

#[derive(Debug, Error)]
pub enum ChoicesError {
    #[error("a tool call names no function")]
    MissingFunction,
    #[error("a custom tool call has no id")]
    MissingCustomId,
    #[error("a custom tool call carries no custom payload")]
    MissingCustom,
    #[error("the tools value is not a list")]
    ToolsNotList,
    #[error("a tool entry is not an object with a function")]
    ToolNotObject,
    #[error("a parallel tool call's arguments name no tool_uses list")]
    ParallelUses,
    #[error("a parallel tool use is not an object with a recipient name")]
    ParallelEntry,
}

#[must_use]
pub fn passed_in_tools(optional_params: &Object) -> bool {
    let Some(tools) = optional_params.get("tools") else {
        return false;
    };
    !tools.is_null()
}

pub fn transform_choices(
    choices: &[ReplyChoice],
    json_mode: bool,
    optional_params: Option<&Object>,
    identifier: fn() -> Uuid,
) -> Result<Vec<TransformedChoice>, ChoicesError> {
    choices
        .iter()
        .map(|choice| transform_choice(choice, json_mode, optional_params, identifier))
        .collect()
}

fn transform_choice(
    choice: &ReplyChoice,
    json_mode: bool,
    optional_params: Option<&Object>,
    identifier: fn() -> Uuid,
) -> Result<TransformedChoice, ChoicesError> {
    let (content, new_tool_calls) = call_candidates(choice, optional_params, identifier)?;
    let json_mode_content = json_mode_content(new_tool_calls.as_deref(), json_mode);
    let (message, intermediate) =
        translated_message(choice, content, new_tool_calls, json_mode_content);
    let (_, native) =
        choice_finish_reason(intermediate.as_deref().or(choice.finish_reason.as_deref()));
    Ok(TransformedChoice {
        index: choice.index.unwrap_or(0),
        finish_reason: final_finish_reason(&message, choice.finish_reason.as_deref()),
        message,
        provider_specific_fields: (!native.is_empty()).then_some(native),
    })
}

fn final_finish_reason(message: &ChoiceMessage, received: Option<&str>) -> String {
    if message.tool_calls.is_some() {
        return String::from("tool_calls");
    }
    received.map_or_else(
        || String::from("stop"),
        |raw| mapped_finish_reason(raw).to_owned(),
    )
}

fn call_candidates(
    choice: &ReplyChoice,
    optional_params: Option<&Object>,
    identifier: fn() -> Uuid,
) -> Result<(Option<String>, Option<Vec<ChoiceToolCall>>), ChoicesError> {
    let content = choice.message.content.clone();
    let Some(calls) = choice.message.tool_calls.as_deref() else {
        return content_fix_up(content, optional_params, identifier);
    };
    let repaired = repair_parallel_tool_calls(convert_tool_calls(calls, identifier)?)?;
    Ok((content, non_empty(repaired)))
}

fn content_fix_up(
    content: Option<String>,
    optional_params: Option<&Object>,
    identifier: fn() -> Uuid,
) -> Result<(Option<String>, Option<Vec<ChoiceToolCall>>), ChoicesError> {
    let fixable = optional_params.is_some_and(passed_in_tools)
        && content.as_deref().is_some_and(|text| !text.is_empty());
    let Some(params) = optional_params.filter(|_| fixable) else {
        return Ok((content, None));
    };
    let tools = params.get("tools").unwrap_or(&Value::Null);
    let converted = content_tool_call(content.as_deref().unwrap_or_default(), tools, identifier)?;
    Ok(converted.map_or((content, None), |call| (None, Some(vec![call]))))
}

fn translated_message(
    choice: &ReplyChoice,
    content: Option<String>,
    new_tool_calls: Option<Vec<ChoiceToolCall>>,
    json_mode_content: Option<String>,
) -> (ChoiceMessage, Option<String>) {
    if let Some(text) = json_mode_content {
        let message = ChoiceMessage {
            role: String::from("assistant"),
            content: Some(text),
            tool_calls: None,
            reasoning_content: None,
        };
        return (message, Some(String::from("stop")));
    }
    let (reasoning_content, message_content) = extract_reasoning_content(&choice.message, content);
    let message = ChoiceMessage {
        role: String::from("assistant"),
        content: message_content,
        tool_calls: new_tool_calls,
        reasoning_content,
    };
    (message, None)
}

fn non_empty(calls: Vec<ChoiceToolCall>) -> Option<Vec<ChoiceToolCall>> {
    if calls.is_empty() { None } else { Some(calls) }
}

fn json_mode_content(calls: Option<&[ChoiceToolCall]>, json_mode: bool) -> Option<String> {
    let [call] = calls? else {
        return None;
    };
    if !json_mode {
        return None;
    }
    let function = call.function.as_ref()?;
    if function.name.as_deref() != Some(JSON_MODE_TOOL) || function.arguments.is_empty() {
        return None;
    }
    Some(function.arguments.clone())
}

fn convert_tool_calls(
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

fn repair_parallel_tool_calls(
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

fn content_tool_call(
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
                if let Some(name) = function.get("name").and_then(Value::as_str) {
                    names.push(name.to_owned());
                }
            }
            Some(_) => return Err(ChoicesError::ToolNotObject),
        }
    }
    Ok(names)
}

fn extract_reasoning_content(
    message: &ReplyMessage,
    content: Option<String>,
) -> (Option<String>, Option<String>) {
    if let Some(reasoning) = &message.reasoning_content {
        return (Some(reasoning.clone()), content);
    }
    if let Some(reasoning) = &message.reasoning {
        return (Some(reasoning.clone()), content);
    }
    let Some(text) = content else {
        return (None, None);
    };
    parse_think_tags(&text)
}

const THINK_OPENINGS: [&str; 3] = ["<think>", "<thinking>", "<budget:thinking>"];
const THINK_CLOSINGS: [&str; 3] = ["</think>", "</thinking>", "</budget:thinking>"];

// _parse_content_for_reasoning: a leading think block becomes reasoning and the rest stays content
fn parse_think_tags(text: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = THINK_OPENINGS
        .into_iter()
        .find_map(|tag| text.strip_prefix(tag))
    else {
        return (None, Some(text.to_owned()));
    };
    let closing = THINK_CLOSINGS
        .into_iter()
        .filter_map(|tag| rest.find(tag))
        .min();
    let Some(end) = closing else {
        return (None, Some(text.to_owned()));
    };
    let reasoning = rest.get(..end).unwrap_or_default().to_owned();
    let remainder = rest.get(end..).unwrap_or_default();
    let content = THINK_CLOSINGS
        .into_iter()
        .find_map(|tag| remainder.strip_prefix(tag))
        .unwrap_or_default()
        .to_owned();
    (Some(reasoning), Some(content))
}
