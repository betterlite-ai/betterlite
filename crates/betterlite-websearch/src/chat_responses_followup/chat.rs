use betterlite_context::agentic_loop_plan::{
    AgenticLoopRequestPatch, FollowupMessage, JsonMap, get_text,
};
use serde_json::value::to_raw_value;

use super::error::FollowupError;
use super::search::{SearchExecutor, SearchSurface, search_batch};
use crate::call_transformation::WebSearchTransformation;
use crate::call_transformation::outcome::search_outcome_text;
use crate::call_transformation::wire::{Call, Results};

const DROPPED_OPTIONAL_PARAMS: [&str; 6] = [
    "tools",
    "tool_choice",
    "extra_body",
    "model_alias_map",
    "stream_response",
    "custom_prompt_dict",
];
const DROPPED_KWARGS: [&str; 7] = [
    "_websearch_interception",
    "acompletion",
    "litellm_logging_obj",
    "custom_llm_provider",
    "model_alias_map",
    "stream_response",
    "custom_prompt_dict",
];
const INTERCEPTION_PREFIX: &str = "_websearch_interception";

pub struct FollowupRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [FollowupMessage],
    pub optional_params: &'request JsonMap,
    pub kwargs: &'request JsonMap,
}

pub async fn build_chat_patch(
    executor: &dyn SearchExecutor,
    tool_calls: &[Call],
    response_format: &str,
    request: &FollowupRequest<'_>,
) -> Result<AgenticLoopRequestPatch, FollowupError> {
    let outcomes = search_batch(executor, tool_calls, request.kwargs, SearchSurface::Chat).await;
    let texts: Vec<String> = outcomes.iter().map(search_outcome_text).collect();
    let (assistant, results) =
        WebSearchTransformation::transform_response(tool_calls, &texts, response_format)?;
    let mut messages: Vec<FollowupMessage> = request.messages.to_vec();
    messages.push(FollowupMessage(to_raw_value(&assistant)?));
    match results {
        Results::OpenAi(tool_messages) => {
            for message in &tool_messages {
                messages.push(FollowupMessage(to_raw_value(message)?));
            }
        }
        Results::Anthropic(message) => messages.push(FollowupMessage(to_raw_value(&message)?)),
    }
    tracing::debug!(
        "WebSearchInterception: Follow-up messages count: {}",
        messages.len()
    );
    let model = chat_model(request.model, request.kwargs);
    tracing::debug!(
        "WebSearchInterception: Built chat completion request patch model={model} messages={}",
        messages.len()
    );
    Ok(AgenticLoopRequestPatch {
        model: Some(model),
        messages: Some(messages),
        tools: None,
        max_tokens: None,
        optional_params: chat_optional_params(request.optional_params),
        kwargs: chat_kwargs(request.kwargs),
    })
}

fn chat_model(model: &str, kwargs: &JsonMap) -> String {
    match get_text(kwargs, "custom_llm_provider") {
        Some(provider) if !model.starts_with(&provider) && !model.contains('/') => {
            format!("{provider}/{model}")
        }
        _ => model.to_owned(),
    }
}

fn chat_optional_params(optional_params: &JsonMap) -> JsonMap {
    let mut clean: JsonMap = optional_params
        .iter()
        .filter(|(key, _value)| !DROPPED_OPTIONAL_PARAMS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    // LiteLLM restores tools only for a non-null value; a null tools reads like an absent one
    if let Some(tools) = optional_params
        .get("tools")
        .filter(|value| value.get() != "null")
    {
        clean.insert("tools".to_owned(), tools.clone());
    }
    clean
}

fn chat_kwargs(kwargs: &JsonMap) -> JsonMap {
    kwargs
        .iter()
        .filter(|(key, _value)| {
            !key.starts_with(INTERCEPTION_PREFIX) && !DROPPED_KWARGS.contains(&key.as_str())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
