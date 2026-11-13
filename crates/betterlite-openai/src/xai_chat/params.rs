use std::fmt::Display;
use std::future::Future;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_fields::nested_filter::filter_value_from_dict;

use crate::xai_chat::error::XaiChatError;

const BASE_OPENAI_PARAMS: [&str; 18] = [
    "logit_bias",
    "logprobs",
    "max_completion_tokens",
    "max_tokens",
    "n",
    "parallel_tool_calls",
    "presence_penalty",
    "response_format",
    "seed",
    "stream",
    "stream_options",
    "temperature",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "user",
    "web_search_options",
];

// The composing layer implements this over B2221; a failed lookup is debugged and the option skipped
pub trait ReasoningLookup<E> {
    fn supports_reasoning(&mut self, model: &str) -> impl Future<Output = Result<bool, E>> + Send;
}

#[must_use]
pub fn supported_with(model: &str, supports_reasoning: bool) -> Vec<&'static str> {
    let mut params = base_params(model);
    if supports_reasoning {
        params.push("reasoning_effort");
    }
    params
}

pub async fn supported_openai_params<C, E>(model: &str, lookup: &mut C) -> Vec<&'static str>
where
    C: ReasoningLookup<E>,
    E: Display,
{
    let mut params = base_params(model);
    match lookup.supports_reasoning(model).await {
        Ok(true) => params.push("reasoning_effort"),
        Ok(false) => {}
        Err(error) => tracing::debug!(%error, "Error checking if model supports reasoning"),
    }
    params
}

fn base_params(model: &str) -> Vec<&'static str> {
    let mut params = BASE_OPENAI_PARAMS.to_vec();
    if !(model.contains("grok-3-mini")
        || model.contains("grok-4")
        || model.contains("grok-code-fast"))
    {
        params.push("stop");
    }
    if !(model.contains("grok-4") || model.contains("grok-code-fast")) {
        params.push("frequency_penalty");
    }
    params
}

pub fn map_openai_params(
    model: &str,
    supports_reasoning: bool,
    requested: &Object,
    optional: &mut Object,
) -> Result<(), XaiChatError> {
    let supported = supported_with(model, supports_reasoning);
    for (name, value) in requested {
        match name.as_str() {
            // the source writes this branch even for a null value, before any supported check
            "max_completion_tokens" => {
                optional.insert(String::from("max_tokens"), value.clone());
            }
            "tools" if !matches!(value, Json::Null) => {
                let Some(tools) = cleaned_tools(value)? else {
                    continue;
                };
                optional.insert(String::from("tools"), Json::List(tools));
            }
            other if supported.contains(&other) && !matches!(value, Json::Null) => {
                optional.insert(name.clone(), value.clone());
            }
            _ => {}
        }
    }
    Ok(())
}

// filter_value_from_dict removes strict at every depth; an empty cleaned list leaves optional untouched
fn cleaned_tools(value: &Json) -> Result<Option<Vec<Json>>, XaiChatError> {
    let Json::List(tools) = value else {
        return Err(XaiChatError::ToolsNotList);
    };
    let mut cleaned = Vec::with_capacity(tools.len());
    for tool in tools {
        let Json::Object(fields) = tool else {
            return Err(XaiChatError::ToolNotObject);
        };
        cleaned.push(Json::Object(filter_value_from_dict(fields, "strict", 0)));
    }
    Ok((!cleaned.is_empty()).then_some(cleaned))
}
