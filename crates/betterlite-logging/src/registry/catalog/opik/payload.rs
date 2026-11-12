use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::fields::payload::StandardLoggingPayload;

// OpikLogger reads LiteLLM's callback kwargs, and only these keys live outside the standard logging object
pub(super) fn callback_kwargs(payload: &StandardLoggingPayload) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    fields.insert("standard_logging_object".to_owned(), payload_json(payload));
    fields.insert(
        "litellm_params".to_owned(),
        Json::Object(IndexMap::from([(
            "metadata".to_owned(),
            request_metadata(payload),
        )])),
    );
    if let Some(provider) = payload.custom_llm_provider.as_ref() {
        fields.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(provider.clone()),
        );
    }
    fields.insert("response_cost".to_owned(), response_cost_json(payload));
    fields
}

// LiteLLM's kwargs carry the LLM cost alone and null when the calculator failed; the payload's response_cost adds the guardrail cost instead
fn response_cost_json(payload: &StandardLoggingPayload) -> Json {
    if payload.cost_failure_info.is_some() {
        return Json::Null;
    }
    let charged = payload
        .cost_breakdown
        .as_ref()
        .and_then(|breakdown| breakdown.guardrail_cost)
        .unwrap_or(Decimal::ZERO);
    payload
        .response_cost
        .checked_sub(charged)
        .map_or_else(|| Json::Null, number)
}

fn payload_json(payload: &StandardLoggingPayload) -> Json {
    match serde_json::to_value(payload).and_then(serde_json::from_value::<Json>) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "the opik standard logging object could not be encoded");
            Json::Object(IndexMap::new())
        }
    }
}

// the opik payload builder reads litellm_params.metadata.opik, which is the request metadata the standard payload nests
fn request_metadata(payload: &StandardLoggingPayload) -> Json {
    match payload.metadata.requester_metadata.as_ref() {
        Some(Json::Object(fields)) => Json::Object(fields.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => Json::Object(IndexMap::new()),
    }
}

// the merged payload builder takes response_obj as the map LiteLLM gets from the response object
pub(super) fn response_fields(payload: &StandardLoggingPayload) -> IndexMap<String, Json> {
    match payload.response.as_ref() {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    }
}

// the subtraction keeps the payload's scale, and LiteLLM's float drops the trailing zeros it adds
fn number(value: Decimal) -> Json {
    match Number::from_str(&value.normalize().to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}

// the logging payload carries epoch seconds, and Opik's batches carry millisecond timestamps
pub(super) fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_i64))
        .and_then(|milliseconds| milliseconds.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_millis)
}
