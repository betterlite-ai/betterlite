use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Number;
use thiserror::Error;

use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

pub const EMBEDDING_CALL_TYPE: &str = "embedding";
pub const EMBEDDING_EVENT: &str = "$ai_embedding";
pub const GENERATION_EVENT: &str = "$ai_generation";

pub type IdGenerator = fn() -> String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PostHogEventPayload {
    pub event: String,
    pub properties: IndexMap<String, Json>,
    pub distinct_id: String,
}

#[derive(Debug, Error)]
pub enum PostHogEventError {
    #[error("litellm_params.metadata must be an object")]
    MetadataType,
    #[error("{field} is not a JSON number")]
    Number { field: &'static str },
}

// PostHogLogger.litellm_internal_fields, the request plumbing that never becomes a user property
const INTERNAL_FIELDS: [&str; 32] = [
    "endpoint",
    "caching_groups",
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_user_id",
    "user_api_key_org_id",
    "user_api_key_team_alias",
    "user_api_key_end_user_id",
    "user_api_key_user_email",
    "user_api_key",
    "user_api_end_user_max_budget",
    "litellm_api_version",
    "global_max_parallel_requests",
    "user_api_key_team_max_budget",
    "user_api_key_team_spend",
    "user_api_key_spend",
    "user_api_key_max_budget",
    "user_api_key_model_max_budget",
    "user_api_key_metadata",
    "headers",
    "litellm_parent_otel_span",
    "requester_ip_address",
    "model_group",
    "model_group_size",
    "deployment",
    "model_info",
    "api_base",
    "hidden_params",
    "parent_run_id",
    "parent_id",
    "user_id",
];

pub fn create_event_payload(
    payload: &StandardLoggingPayload,
    ids: IdGenerator,
) -> Result<PostHogEventPayload, PostHogEventError> {
    let event = event_name(&payload.call_type);
    let metadata = metadata_of(payload)?;
    Ok(PostHogEventPayload {
        event: event.to_owned(),
        properties: create_properties(payload, metadata, event)?,
        distinct_id: distinct_id(payload, metadata, ids),
    })
}
fn event_name(call_type: &str) -> &'static str {
    if call_type == EMBEDDING_CALL_TYPE {
        EMBEDDING_EVENT
    } else {
        GENERATION_EVENT
    }
}

const fn metadata_of(
    payload: &StandardLoggingPayload,
) -> Result<Option<&IndexMap<String, Json>>, PostHogEventError> {
    match &payload.metadata.requester_metadata {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(PostHogEventError::MetadataType)
        }
    }
}
fn create_properties(
    payload: &StandardLoggingPayload,
    metadata: Option<&IndexMap<String, Json>>,
    event: &str,
) -> Result<IndexMap<String, Json>, PostHogEventError> {
    let generation = event == GENERATION_EVENT;
    let mut properties = model_properties(payload, generation)?;
    if payload.status == LoggingStatus::Failure {
        properties.insert("$ai_is_error".to_owned(), Json::Bool(true));
        if let Some(error_str) = &payload.error_str {
            properties.insert("$ai_error".to_owned(), Json::Text(error_str.clone()));
        }
    }
    properties.insert(
        "$ai_trace_id".to_owned(),
        Json::Text(payload.trace_id.clone()),
    );
    properties.insert("$ai_span_id".to_owned(), Json::Text(payload.id.clone()));
    if let Some(parent) = parent_id(metadata) {
        properties.insert("$ai_parent_id".to_owned(), parent.clone());
    }
    // LiteLLM applies the caller's metadata last, so a custom key wins over a built-in one
    for (key, value) in metadata.into_iter().flatten() {
        if !INTERNAL_FIELDS.contains(&key.as_str()) {
            properties.insert(key.clone(), value.clone());
        }
    }
    Ok(properties)
}
fn model_properties(
    payload: &StandardLoggingPayload,
    generation: bool,
) -> Result<IndexMap<String, Json>, PostHogEventError> {
    let mut properties = IndexMap::new();
    properties.insert("$ai_model".to_owned(), Json::Text(payload.model.clone()));
    properties.insert(
        "$ai_provider".to_owned(),
        Json::Text(payload.custom_llm_provider.clone().unwrap_or_default()),
    );
    if let Some(messages) = &payload.messages {
        properties.insert("$ai_input".to_owned(), messages.clone());
    }
    if generation && let Some(response) = &payload.response {
        properties.insert("$ai_output_choices".to_owned(), response.clone());
    }
    properties.insert(
        "$ai_input_tokens".to_owned(),
        Json::Number(Number::from(payload.prompt_tokens)),
    );
    if generation {
        properties.insert(
            "$ai_output_tokens".to_owned(),
            Json::Number(Number::from(payload.completion_tokens)),
        );
    }
    properties.insert(
        "$ai_total_cost_usd".to_owned(),
        number(payload.response_cost, "$ai_total_cost_usd")?,
    );
    properties.insert(
        "$ai_latency".to_owned(),
        number(payload.response_time, "$ai_latency")?,
    );
    Ok(properties)
}
fn number(value: Decimal, field: &'static str) -> Result<Json, PostHogEventError> {
    match Number::from_str(&value.to_string()) {
        Ok(number) => Ok(Json::Number(number)),
        Err(_source) => Err(PostHogEventError::Number { field }),
    }
}
fn parent_id(metadata: Option<&IndexMap<String, Json>>) -> Option<&Json> {
    let fields = metadata?;
    ["parent_run_id", "parent_id"]
        .into_iter()
        .find_map(|name| fields.get(name).filter(|value| is_present(value)))
}

const fn is_present(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Text(text) => !text.is_empty(),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => true,
    }
}
fn distinct_id(
    payload: &StandardLoggingPayload,
    metadata: Option<&IndexMap<String, Json>>,
    ids: IdGenerator,
) -> String {
    let preferred = metadata
        .and_then(|fields| fields.get("user_id"))
        .and_then(text_value);
    if let Some(user) = preferred {
        return user.to_owned();
    }
    if let Some(end_user) = payload
        .end_user
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return end_user.to_owned();
    }
    if !payload.trace_id.is_empty() {
        return payload.trace_id.clone();
    }
    ids()
}
fn text_value(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) if !text.is_empty() => Some(text),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}
