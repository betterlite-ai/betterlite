use super::projections::{ChatChoice, ChatToolCall, ResponsesOutputItem, ToolCallResponse};
use crate::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use std::slice::from_ref;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NormalizedToolCall {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Object,
}

struct ParsedToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: Vec<Object>,
}

// LiteLLM tries the chat section, then the Responses output, then the Anthropic content, and stops at the first one that yields calls.
#[must_use]
pub fn get_tool_calls_from_response(
    response: &ToolCallResponse,
    include_all_choices: bool,
) -> Vec<NormalizedToolCall> {
    let chat = chat_completion_tool_calls(response, include_all_choices);
    if !chat.is_empty() {
        return chat;
    }
    let responses = responses_api_tool_calls(response);
    if !responses.is_empty() {
        return responses;
    }
    anthropic_messages_tool_calls(response)
}

#[must_use]
pub fn chat_completion_tool_calls(
    response: &ToolCallResponse,
    include_all_choices: bool,
) -> Vec<NormalizedToolCall> {
    let mut parses = Vec::new();
    for choice in selected_choices(&response.choices, include_all_choices) {
        let Some(message) = choice.message.as_ref() else {
            continue;
        };
        for tool_call in &message.tool_calls {
            if let Some(parsed) = parsed_chat_tool_call(tool_call) {
                parses.push(parsed);
            }
        }
    }
    normalized_tool_calls(&parses)
}

#[must_use]
pub fn responses_api_tool_calls(response: &ToolCallResponse) -> Vec<NormalizedToolCall> {
    let parses: Vec<ParsedToolCall> = response
        .output
        .iter()
        .filter(|item| item.kind.as_deref() == Some("function_call"))
        .map(parsed_response_tool_call)
        .collect();
    normalized_tool_calls(&parses)
}

#[must_use]
pub fn anthropic_messages_tool_calls(response: &ToolCallResponse) -> Vec<NormalizedToolCall> {
    let mut calls = Vec::new();
    for block in &response.content {
        if block.kind.as_deref() != Some("tool_use") {
            continue;
        }
        calls.push(NormalizedToolCall {
            id: block.id.clone(),
            name: block.name.clone(),
            arguments: block
                .input
                .as_ref()
                .and_then(Json::as_object)
                .cloned()
                .unwrap_or_default(),
        });
    }
    calls
}

const fn selected_choices(choices: &[ChatChoice], include_all_choices: bool) -> &[ChatChoice] {
    if include_all_choices {
        return choices;
    }
    match choices.first() {
        Some(first) => from_ref(first),
        None => &[],
    }
}

fn parsed_chat_tool_call(tool_call: &ChatToolCall) -> Option<ParsedToolCall> {
    let function = tool_call.function.as_ref()?;
    let id = tool_call
        .id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned);
    Some(ParsedToolCall {
        id,
        name: function.name.clone(),
        arguments: arguments::parse(
            function.arguments.as_ref(),
            function.name.as_deref(),
            "chat completions",
        ),
    })
}

fn parsed_response_tool_call(item: &ResponsesOutputItem) -> ParsedToolCall {
    let raw_id = item.call_id.as_deref().or(item.id.as_deref());
    let id = raw_id.filter(|id| !id.is_empty()).map(ToOwned::to_owned);
    ParsedToolCall {
        id,
        name: item.name.clone(),
        arguments: arguments::parse(
            item.arguments.as_ref(),
            item.name.as_deref(),
            "responses API",
        ),
    }
}

fn normalized_tool_calls(parses: &[ParsedToolCall]) -> Vec<NormalizedToolCall> {
    let counts: Vec<(Option<&str>, usize)> = parses
        .iter()
        .map(|parse| (parse.id.as_deref(), parse.arguments.len()))
        .collect();
    let id_groups = identity::split_tool_call_ids(&counts);
    let mut calls = Vec::new();
    for (parse, ids) in parses.iter().zip(id_groups) {
        for (id, arguments) in ids.into_iter().zip(&parse.arguments) {
            calls.push(NormalizedToolCall {
                id,
                name: parse.name.clone(),
                arguments: arguments.clone(),
            });
        }
    }
    calls
}

mod arguments;
mod identity;
mod tests;
