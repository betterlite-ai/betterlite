pub mod error;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;

use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use crate::zerobus::row::error::TraceRowError;

const MICROSECONDS_PER_SECOND: i64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TraceRow {
    pub id: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub litellm_call_id: Option<String>,
    pub call_type: Option<String>,
    pub status: Option<String>,
    pub model: Option<String>,
    pub model_group: Option<String>,
    pub model_id: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub api_base: Option<String>,
    pub stream: Option<bool>,
    pub cache_hit: Option<bool>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub completion_start_time: Option<i64>,
    pub response_time: Option<f64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub saved_cache_cost: Option<Decimal>,
    pub api_key_hash: Option<String>,
    pub api_key_alias: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub user_id: Option<String>,
    pub org_id: Option<String>,
    pub end_user: Option<String>,
    pub requester_ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub request_tags: Option<String>,
    pub messages: Option<String>,
    pub response: Option<String>,
    pub error_str: Option<String>,
    pub error_information: Option<String>,
    pub metadata: Option<String>,
    pub model_parameters: Option<String>,
    pub hidden_params: Option<String>,
    pub guardrail_information: Option<String>,
    pub cost_breakdown: Option<String>,
}

fn text(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

const fn flag(value: Option<&Json>) -> Option<bool> {
    match value {
        Some(Json::Bool(flag)) => Some(*flag),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn decimal(value: Option<&Json>) -> Option<Decimal> {
    let Some(Json::Number(number)) = value else {
        return None;
    };
    number.to_string().parse::<Decimal>().ok()
}

fn number_as_f64(value: &Json) -> Option<f64> {
    match value {
        Json::Number(number) => number.as_f64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn timestamp_micros(value: Option<&Json>) -> Option<i64> {
    let seconds = decimal(value)?;
    if seconds <= Decimal::ZERO {
        return None;
    }
    let micros = seconds.checked_mul(Decimal::from(MICROSECONDS_PER_SECOND))?;
    micros.to_i64()
}

fn json_value(value: Option<&Json>) -> Result<Option<String>, TraceRowError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(nested) => Ok(Some(
            safe_dumps(nested, DEFAULT_MAX_RECURSE_DEPTH, None)
                .map_err(TraceRowError::NestedField)?,
        )),
    }
}

fn metadata_text(payload: &IndexMap<String, Json>, key: &str) -> Option<String> {
    let Some(Json::Object(fields)) = payload.get("metadata") else {
        return None;
    };
    text(fields.get(key))
}

// the fixed row names every Delta column once, so splitting it would only scatter one projection
#[expect(
    clippy::too_many_lines,
    reason = "one initializer per Delta column keeps the fixed row readable"
)]
pub fn trace_row(payload: &IndexMap<String, Json>) -> Result<TraceRow, TraceRowError> {
    Ok(TraceRow {
        id: text(payload.get("id")),
        trace_id: text(payload.get("trace_id")),
        session_id: text(payload.get("session_id")),
        litellm_call_id: text(payload.get("litellm_call_id")),
        call_type: text(payload.get("call_type")),
        status: text(payload.get("status")),
        model: text(payload.get("model")),
        model_group: text(payload.get("model_group")),
        model_id: text(payload.get("model_id")),
        custom_llm_provider: text(payload.get("custom_llm_provider")),
        api_base: text(payload.get("api_base")),
        stream: flag(payload.get("stream")),
        cache_hit: flag(payload.get("cache_hit")),
        start_time: timestamp_micros(payload.get("startTime")),
        end_time: timestamp_micros(payload.get("endTime")),
        completion_start_time: timestamp_micros(payload.get("completionStartTime")),
        response_time: payload.get("response_time").and_then(number_as_f64),
        prompt_tokens: decimal(payload.get("prompt_tokens")).and_then(|number| number.to_i64()),
        completion_tokens: decimal(payload.get("completion_tokens"))
            .and_then(|number| number.to_i64()),
        total_tokens: decimal(payload.get("total_tokens")).and_then(|number| number.to_i64()),
        response_cost: decimal(payload.get("response_cost")),
        saved_cache_cost: decimal(payload.get("saved_cache_cost")),
        api_key_hash: metadata_text(payload, "user_api_key_hash"),
        api_key_alias: metadata_text(payload, "user_api_key_alias"),
        team_id: metadata_text(payload, "user_api_key_team_id"),
        team_alias: metadata_text(payload, "user_api_key_team_alias"),
        user_id: metadata_text(payload, "user_api_key_user_id"),
        org_id: metadata_text(payload, "user_api_key_org_id"),
        end_user: text(payload.get("end_user")),
        requester_ip_address: text(payload.get("requester_ip_address")),
        user_agent: text(payload.get("user_agent")),
        request_tags: json_value(payload.get("request_tags"))?,
        messages: json_value(payload.get("messages"))?,
        response: json_value(payload.get("response"))?,
        error_str: text(payload.get("error_str")),
        error_information: json_value(payload.get("error_information"))?,
        metadata: json_value(payload.get("metadata"))?,
        model_parameters: json_value(payload.get("model_parameters"))?,
        hidden_params: json_value(payload.get("hidden_params"))?,
        guardrail_information: json_value(payload.get("guardrail_information"))?,
        cost_breakdown: json_value(payload.get("cost_breakdown"))?,
    })
}
