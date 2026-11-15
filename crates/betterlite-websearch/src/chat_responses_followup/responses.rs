use betterlite_context::agentic_loop_plan::{
    AgenticLoopRequestPatch, FollowupMessage, JsonMap, get_text,
};
use serde::Serialize;
use serde_json::value::{RawValue, to_raw_value};

use super::error::FollowupError;
use super::search::{SearchExecutor, SearchSurface, search_batch};
use crate::call_transformation::outcome::search_outcome_text;
use crate::call_transformation::wire::Call;
use crate::tools::LITELLM_WEB_SEARCH_TOOL_NAME;

const DROPPED_OPTIONAL_PARAMS: [&str; 6] = [
    "tools",
    "tool_choice",
    "stream",
    "model_alias_map",
    "stream_response",
    "custom_prompt_dict",
];
const DROPPED_KWARGS: [&str; 5] = [
    "_agentic_loop_api_surface",
    "litellm_logging_obj",
    "acompletion",
    "custom_llm_provider",
    "model_alias_map",
];
const INTERCEPTION_PREFIX: &str = "_websearch_interception";

#[derive(Debug, Clone)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<FollowupMessage>),
}

pub struct ResponsesRequest<'request> {
    pub model: &'request str,
    pub input: &'request ResponsesInput,
    pub optional_params: &'request JsonMap,
    pub kwargs: &'request JsonMap,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FollowupItem {
    FunctionCall {
        call_id: Option<String>,
        name: &'static str,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: Option<String>,
        output: String,
    },
}

#[derive(Debug, Serialize)]
struct UserItem<'text> {
    role: &'static str,
    content: &'text str,
}

pub async fn build_responses_patch(
    executor: &dyn SearchExecutor,
    tool_calls: &[Call],
    request: &ResponsesRequest<'_>,
) -> Result<AgenticLoopRequestPatch, FollowupError> {
    let outcomes = search_batch(
        executor,
        tool_calls,
        request.kwargs,
        SearchSurface::Responses,
    )
    .await;
    let texts: Vec<String> = outcomes.iter().map(search_outcome_text).collect();
    let mut messages = normalized_input(request.input)?;
    for (call, text) in tool_calls.iter().zip(&texts) {
        messages.push(FollowupMessage(to_raw_value(
            &FollowupItem::FunctionCall {
                call_id: call.call_id.clone().flatten(),
                name: LITELLM_WEB_SEARCH_TOOL_NAME,
                arguments: call.arguments.clone().unwrap_or_default(),
            },
        )?));
        messages.push(FollowupMessage(to_raw_value(
            &FollowupItem::FunctionCallOutput {
                call_id: call.call_id.clone().flatten(),
                output: text.clone(),
            },
        )?));
    }
    let model = responses_model(request.model, request.kwargs);
    tracing::debug!(
        "WebSearchInterception: Built responses request patch model={model} input_items={} searches={}",
        messages.len(),
        texts.len()
    );
    Ok(AgenticLoopRequestPatch {
        model: Some(model),
        messages: Some(messages),
        tools: responses_tools(request.optional_params),
        max_tokens: None,
        optional_params: responses_optional_params(request.optional_params),
        kwargs: responses_kwargs(request.kwargs),
    })
}

fn normalized_input(input: &ResponsesInput) -> Result<Vec<FollowupMessage>, FollowupError> {
    match input {
        ResponsesInput::Text(text) => Ok(vec![FollowupMessage(to_raw_value(&UserItem {
            role: "user",
            content: text,
        })?)]),
        ResponsesInput::Items(items) => Ok(items.clone()),
    }
}

fn responses_tools(optional_params: &JsonMap) -> Option<Vec<FollowupMessage>> {
    let tools: Vec<Box<RawValue>> =
        serde_json::from_str(optional_params.get("tools")?.get()).ok()?;
    Some(tools.into_iter().map(FollowupMessage).collect())
}

fn responses_model(model: &str, kwargs: &JsonMap) -> String {
    match get_text(kwargs, "custom_llm_provider") {
        Some(provider) if !model.contains('/') => format!("{provider}/{model}"),
        _ => model.to_owned(),
    }
}

fn responses_optional_params(optional_params: &JsonMap) -> JsonMap {
    optional_params
        .iter()
        .filter(|(key, _value)| !DROPPED_OPTIONAL_PARAMS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn responses_kwargs(kwargs: &JsonMap) -> JsonMap {
    kwargs
        .iter()
        .filter(|(key, _value)| {
            !key.starts_with(INTERCEPTION_PREFIX) && !DROPPED_KWARGS.contains(&key.as_str())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
