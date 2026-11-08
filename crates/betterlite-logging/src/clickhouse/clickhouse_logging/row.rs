use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::context::LensAnalysis;
use super::convert::{
    cache_tokens, find_traceparent, int_value, json_value, request_tags, session_id,
    strip_cache_hit_suffix, to_ms,
};
use super::payload::{CallbackKwargs, StandardLoggingPayload};
use crate::trace_types::SpendLogRecord;

const TEAM_ID_KEYS: [&str; 2] = ["user_api_key_team_id", "team_id"];
const TEAM_ALIAS_KEYS: [&str; 2] = ["user_api_key_team_alias", "team_alias"];

#[derive(Clone, Copy)]
pub struct Projection<'payload> {
    pub payload: &'payload StandardLoggingPayload,
    pub kwargs: &'payload CallbackKwargs,
    pub message_logging_disabled: bool,
    pub lens: &'payload LensAnalysis,
}

fn text_value(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            String::new()
        }
    }
}

fn text_field(metadata: Option<&IndexMap<String, Json>>, key: &str) -> String {
    metadata
        .and_then(|fields| fields.get(key))
        .map_or_else(String::new, text_value)
}

fn first_text(metadata: Option<&IndexMap<String, Json>>, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|key| metadata?.get(*key))
        .map(text_value)
        .find(|text| !text.is_empty())
        .unwrap_or_default()
}

fn usage_object<'payload>(
    metadata: Option<&'payload IndexMap<String, Json>>,
    hidden_params: Option<&'payload IndexMap<String, Json>>,
) -> Option<&'payload IndexMap<String, Json>> {
    match metadata.and_then(|fields| fields.get("usage_object")) {
        Some(Json::Object(usage)) => Some(usage),
        _ => match hidden_params.and_then(|fields| fields.get("usage_object")) {
            Some(Json::Object(usage)) => Some(usage),
            _ => None,
        },
    }
}

fn tokens(payload: &StandardLoggingPayload) -> (i64, i64) {
    usage_object(payload.metadata.as_ref(), payload.hidden_params.as_ref())
        .map_or((0, 0), cache_tokens)
}

fn end_user(payload: &StandardLoggingPayload) -> String {
    if let Some(end_user) = payload.end_user.as_deref().filter(|text| !text.is_empty()) {
        return end_user.to_owned();
    }
    text_field(payload.metadata.as_ref(), "user_api_key_end_user_id")
}

fn content(projection: Projection<'_>, value: Option<&Json>) -> String {
    if projection.message_logging_disabled {
        return String::new();
    }
    json_value(value)
}

fn completion_start_time(payload: &StandardLoggingPayload) -> Option<i64> {
    match to_ms(payload.completion_start_time.as_ref()) {
        Some(0) => None,
        millis => millis,
    }
}

fn row_metadata(metadata: Option<&IndexMap<String, Json>>, lens: &LensAnalysis) -> String {
    let mut merged = metadata.cloned().unwrap_or_default();
    merged.insert(
        "litellm_lens_internal".to_owned(),
        Json::Bool(lens.is_lens_analysis()),
    );
    serde_json::to_string(&merged).unwrap_or_default()
}

#[must_use]
pub fn spend_log_row_from_payload(projection: Projection<'_>) -> SpendLogRecord {
    let payload = projection.payload;
    SpendLogRecord {
        request_id: payload.id.clone().unwrap_or_default(),
        response_id: strip_cache_hit_suffix(payload.id.as_deref().unwrap_or_default()),
        call_type: payload.call_type.clone().unwrap_or_default(),
        api_key: text_field(payload.metadata.as_ref(), "user_api_key_hash"),
        key_alias: text_field(payload.metadata.as_ref(), "user_api_key_alias"),
        team_id: first_text(payload.metadata.as_ref(), &TEAM_ID_KEYS),
        team_alias: first_text(payload.metadata.as_ref(), &TEAM_ALIAS_KEYS),
        organization_id: text_field(payload.metadata.as_ref(), "user_api_key_org_id"),
        user: text_field(payload.metadata.as_ref(), "user_api_key_user_id"),
        end_user: end_user(payload),
        model: payload.model.clone().unwrap_or_default(),
        model_group: payload.model_group.clone().unwrap_or_default(),
        model_id: payload.model_id.clone().unwrap_or_default(),
        custom_llm_provider: payload.custom_llm_provider.clone().unwrap_or_default(),
        api_base: payload.api_base.clone().unwrap_or_default(),
        spend: payload.response_cost.unwrap_or_default(),
        prompt_tokens: int_value(payload.prompt_tokens.as_ref()),
        completion_tokens: int_value(payload.completion_tokens.as_ref()),
        total_tokens: int_value(payload.total_tokens.as_ref()),
        cache_read_tokens: tokens(payload).0,
        cache_write_tokens: tokens(payload).1,
        start_time: to_ms(payload.start_time.as_ref()).unwrap_or_default(),
        end_time: to_ms(payload.end_time.as_ref()).unwrap_or_default(),
        completion_start_time: completion_start_time(payload),
        status: payload.status.clone().unwrap_or_default(),
        error_str: payload.error_str.clone().unwrap_or_default(),
        cache_hit: payload.cache_hit == Some(true),
        session_id: session_id(payload, projection.kwargs),
        trace_id: find_traceparent(payload.metadata.as_ref(), projection.kwargs).0,
        span_id: find_traceparent(payload.metadata.as_ref(), projection.kwargs).1,
        request_tags: request_tags(payload.request_tags.as_ref()),
        metadata: row_metadata(payload.metadata.as_ref(), projection.lens),
        messages: content(projection, payload.messages.as_ref()),
        response: content(projection, payload.response.as_ref()),
    }
}

mod tests;
