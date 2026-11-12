use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use betterlite_prompts::reasoning_fields::content_split::parse_content_for_reasoning;

use crate::chat::choice_finish_reason;
use crate::chat_choices::tool_calls::{
    content_tool_call, convert_tool_calls, repair_parallel_tool_calls,
};
use crate::chat_choices::wire::{
    ChoiceMessage, ChoiceToolCall, ProviderField, ReplyChoice, ReplyMessage, TransformedChoice,
};
use crate::openai_base_config::Object;
use crate::text_completion_policy::reply::mapped_finish_reason;

const JSON_MODE_TOOL: &str = "json_tool_call";

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

// _extract_reasoning_content: a reasoning key wins even when null, else the content's leading think block
fn extract_reasoning_content(
    message: &ReplyMessage,
    content: Option<String>,
) -> (Option<String>, Option<String>) {
    let provided = [&message.reasoning_content, &message.reasoning]
        .into_iter()
        .find(|field| !matches!(field, ProviderField::Absent));
    match provided {
        Some(ProviderField::Text(reasoning)) => (Some(reasoning.clone()), content),
        Some(ProviderField::Null | ProviderField::Absent) => (None, content),
        None => parse_content_for_reasoning(content.as_deref()),
    }
}
