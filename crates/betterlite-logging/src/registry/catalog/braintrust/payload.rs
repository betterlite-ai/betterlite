use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::braintrust_logging::payload::{ResponseObject, SuccessEvent};
use crate::fields::payload::StandardLoggingPayload;

#[derive(Debug, thiserror::Error)]
pub(super) enum PayloadError {
    #[error("the standard logging payload could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the standard logging payload {field} is not a valid timestamp")]
    Timestamp { field: &'static str },
}

#[derive(Clone, Copy)]
enum ResponseKind {
    Model,
    TextCompletion,
    Embedding,
    Image,
}

struct ResponseBody {
    kind: ResponseKind,
    body: Json,
}

impl ResponseBody {
    const fn object(&self) -> ResponseObject<'_> {
        match self.kind {
            ResponseKind::Model => ResponseObject::Model(&self.body),
            ResponseKind::TextCompletion => ResponseObject::TextCompletion(&self.body),
            ResponseKind::Embedding => ResponseObject::Embedding(&self.body),
            ResponseKind::Image => ResponseObject::Image(&self.body),
        }
    }
}

// the fields of BraintrustLogger.log_success_event's arguments, owned so one SuccessEvent can borrow all of them
pub(super) struct Prepared {
    call_id: Option<Json>,
    messages: Option<Json>,
    standard_logging_object: Json,
    metadata: Json,
    response_cost: Option<Json>,
    call_type: String,
    response: Option<ResponseBody>,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    completion_start_time: DateTime<Utc>,
}

impl Prepared {
    pub(super) fn new(payload: &StandardLoggingPayload) -> Result<Self, PayloadError> {
        Ok(Self {
            call_id: payload.litellm_call_id.clone().map(Json::Text),
            messages: payload.messages.clone(),
            standard_logging_object: standard_logging_object(payload)?,
            metadata: metadata_object(payload),
            response_cost: total_cost(payload),
            call_type: payload.call_type.clone(),
            response: classify_response(payload),
            start_time: utc_at(payload.start_time)
                .ok_or(PayloadError::Timestamp { field: "startTime" })?,
            end_time: utc_at(payload.end_time)
                .ok_or(PayloadError::Timestamp { field: "endTime" })?,
            completion_start_time: utc_at(payload.completion_start_time).ok_or(
                PayloadError::Timestamp {
                    field: "completionStartTime",
                },
            )?,
        })
    }

    pub(super) fn event(&self) -> SuccessEvent<'_> {
        SuccessEvent {
            call_id: self.call_id.as_ref(),
            messages: self.messages.as_ref(),
            standard_logging_object: Some(&self.standard_logging_object),
            metadata: Some(&self.metadata),
            response_cost: self.response_cost.as_ref(),
            call_type: Some(self.call_type.as_str()),
            response: self.response.as_ref().map(ResponseBody::object),
            // the payload keeps no provider-handoff instant, and LiteLLM sets time to first token only when it has one
            api_call_start_time: None,
            completion_start_time: Some(self.completion_start_time),
            start_time: self.start_time,
            end_time: self.end_time,
        }
    }
}

// the standard logging object LiteLLM hands the logger is the payload enum, and braintrust posts it as the event metadata
fn standard_logging_object(payload: &StandardLoggingPayload) -> Result<Json, PayloadError> {
    serde_json::to_value(payload)
        .and_then(serde_json::from_value::<Json>)
        .map_err(PayloadError::Encode)
}

// a request's own metadata rides nested in the standard payload, and braintrust reads project_name and the span keys flat
fn metadata_object(payload: &StandardLoggingPayload) -> Json {
    let encoded = serde_json::to_value(&payload.metadata).and_then(serde_json::from_value::<Json>);
    let Ok(Json::Object(mut fields)) = encoded else {
        tracing::warn!("the braintrust callback metadata could not be encoded");
        return Json::Object(IndexMap::new());
    };
    if let Some(Json::Object(requester)) = payload.metadata.requester_metadata.as_ref() {
        fields.extend(requester.clone());
    }
    Json::Object(fields)
}

// LiteLLM picks the response branch by the response object's class, and the payload keeps the call type and the body
fn classify_response(payload: &StandardLoggingPayload) -> Option<ResponseBody> {
    let Some(response @ Json::Object(fields)) = payload.response.as_ref() else {
        return None;
    };
    let kind = match payload.call_type.as_str() {
        "embedding" | "aembedding" => ResponseKind::Embedding,
        "text_completion" | "atext_completion" => ResponseKind::TextCompletion,
        "image_generation" | "aimage_generation" | "image_edit" | "aimage_edit" => {
            ResponseKind::Image
        }
        _ if fields.contains_key("choices") => ResponseKind::Model,
        _ => return None,
    };
    Some(ResponseBody {
        kind,
        body: response.clone(),
    })
}

// LiteLLM's kwargs carry the LLM cost alone and null when the calculator failed; the payload's response_cost adds the guardrail cost instead
fn total_cost(payload: &StandardLoggingPayload) -> Option<Json> {
    if payload.cost_failure_info.is_some() {
        return None;
    }
    let charged = payload
        .cost_breakdown
        .as_ref()
        .and_then(|fields| fields.guardrail_cost);
    let cost = match charged {
        Some(amount) => payload.response_cost.checked_sub(amount)?,
        None => payload.response_cost,
    };
    Some(number(cost))
}

// the subtraction keeps the payload's scale, and LiteLLM's float drops the trailing zeros it adds
fn number(value: Decimal) -> Json {
    match Number::from_str(&value.normalize().to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}

fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_000_i64))
        .and_then(|micros| micros.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_micros)
}
