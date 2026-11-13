use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::langfuse_projections::steering::truthiness;
use crate::newrelic_extraction::ids::UuidSource;
use crate::newrelic_extraction::sources::EventSources;
use crate::python_json;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

pub fn completion_id(sources: &EventSources<'_>, ids: &dyn UuidSource) -> String {
    if let Some(id) = sources.response.get("id").filter(|value| truthiness(value)) {
        return match id {
            Json::Text(text) => text.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                python_json::dumps(id)
            }
        };
    }
    if let Some(call_id) = sources.kwargs.litellm_call_id.filter(|id| !id.is_empty()) {
        return call_id.to_owned();
    }
    ids.next_id().to_string()
}

pub fn vendor(sources: &EventSources<'_>) -> String {
    if let Some(provider) = sources
        .standard_logging_object
        .and_then(|payload| payload.custom_llm_provider.as_deref())
        .filter(|provider| !provider.is_empty())
    {
        return provider.to_owned();
    }
    sources
        .kwargs
        .custom_llm_provider
        .filter(|provider| !provider.is_empty())
        .map_or_else(|| "litellm".to_owned(), str::to_owned)
}

pub fn model_names(sources: &EventSources<'_>) -> (String, String) {
    let payload_model = sources
        .standard_logging_object
        .map(|payload| payload.model.as_str())
        .filter(|model| !model.is_empty());
    let chosen_model =
        payload_model.or_else(|| sources.kwargs.model.filter(|model| !model.is_empty()));
    let request_model = chosen_model.map_or_else(|| "unknown".to_owned(), str::to_owned);
    let response_model = match sources
        .response
        .get("model")
        .filter(|value| truthiness(value))
    {
        Some(Json::Text(response_text)) => response_text.clone(),
        Some(response_value) => python_json::dumps(response_value),
        None => request_model.clone(),
    };
    (request_model, response_model)
}

#[must_use]
pub fn usage(sources: &EventSources<'_>) -> Usage {
    if let Some(payload) = sources.standard_logging_object {
        return Usage {
            prompt_tokens: payload.prompt_tokens,
            completion_tokens: payload.completion_tokens,
            total_tokens: payload.total_tokens,
        };
    }
    let Some(Json::Object(fields)) = sources.response.get("usage") else {
        return Usage::default();
    };
    Usage {
        prompt_tokens: token_count(fields, "prompt_tokens"),
        completion_tokens: token_count(fields, "completion_tokens"),
        total_tokens: token_count(fields, "total_tokens"),
    }
}

fn token_count(fields: &IndexMap<String, Json>, name: &str) -> i64 {
    match fields.get(name) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}

#[must_use]
pub fn finish_reason(sources: &EventSources<'_>) -> String {
    let choices = match sources.response.get("choices") {
        Some(Json::List(choices)) => choices,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => {
            return "unknown".to_owned();
        }
    };
    let Some(Json::Object(first)) = choices.first() else {
        return "unknown".to_owned();
    };
    match first.get("finish_reason").filter(|value| truthiness(value)) {
        Some(Json::Text(reason)) => reason.clone(),
        Some(reason) => python_json::dumps(reason),
        None => "unknown".to_owned(),
    }
}
