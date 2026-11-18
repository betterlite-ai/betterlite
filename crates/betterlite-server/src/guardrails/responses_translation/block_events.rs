#![expect(
    clippy::disallowed_types,
    reason = "Responses stream data is an open wire boundary"
)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_synthetic_events::{
    builder::build_synthetic_response_events,
    stamping::{CostTarget, ResponsesCostCalculator},
};
use betterlite_guardrails::guardrail_message_scope::{
    Object,
    blocked_usage::{OriginalResponse, blocked_responses_stream_usage},
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};
use std::convert::Infallible;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockStamp {
    pub response_id: String,
    pub message_id: String,
    pub created_at: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum BlockEventError {
    #[error("response serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("synthetic response events failed: {0}")]
    Synthetic(#[from] betterlite_core::api::response_synthetic_events::error::SyntheticEventsError),
}

fn to_guardrail(value: &Value) -> GuardrailValue {
    match value {
        Value::Null => GuardrailValue::Null,
        Value::Bool(value) => GuardrailValue::Boolean(*value),
        Value::Number(value) => value.as_i64().map_or_else(
            || {
                value.as_u64().map_or_else(
                    || GuardrailValue::Decimal(value.as_f64().unwrap_or_default()),
                    GuardrailValue::UnsignedInteger,
                )
            },
            GuardrailValue::SignedInteger,
        ),
        Value::String(value) => GuardrailValue::Text(value.clone()),
        Value::Array(values) => GuardrailValue::Array(values.iter().map(to_guardrail).collect()),
        Value::Object(values) => GuardrailValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), to_guardrail(value)))
                .collect(),
        ),
    }
}

fn original(value: Option<&Value>) -> OriginalResponse {
    match value.and_then(Value::as_object) {
        Some(value) => OriginalResponse::Response(
            value
                .iter()
                .map(|(key, value)| (key.clone(), to_guardrail(value)))
                .collect::<Object>(),
        ),
        None => OriginalResponse::Absent,
    }
}

struct NoCost;
impl ResponsesCostCalculator for NoCost {
    type Error = Infallible;
    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error> {
        Ok(None)
    }
}

pub fn blocked_output_item(message: &str, stamp: &BlockStamp) -> Value {
    json!({"type":"message","id":stamp.message_id,"status":"completed","role":"assistant","content":[{"type":"output_text","text":message,"annotations":[]}]})
}

fn response(id: &str, model: &str, created_at: i64, items: Vec<Value>, usage: Value) -> Value {
    json!({"id":id,"object":"response","created_at":created_at,"model":model,"output":items,"status":"completed","usage":usage})
}

pub fn build_blocked_response(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    stamp: &BlockStamp,
) -> Value {
    let usage = serde_json::to_value(blocked_responses_stream_usage(&original(original_response)))
        .unwrap_or_else(|_| json!({}));
    response(
        &stamp.response_id,
        model,
        stamp.created_at,
        vec![blocked_output_item(message, stamp)],
        usage,
    )
}

fn continuation_identity(
    model: &str,
    events: Option<&[Value]>,
    stamp: &BlockStamp,
) -> (String, String, i64) {
    let mut id = None;
    let mut streamed_model = None;
    let mut highest = None;
    if let Some(events) = events {
        for event in events {
            if let Some(response) = event.get("response") {
                if id.is_none() {
                    id = response
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned);
                }
                if streamed_model.is_none() {
                    streamed_model = response
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned);
                }
            }
            if let Some(index) = event.get("output_index").and_then(Value::as_i64) {
                highest = Some(highest.map_or(index, |current: i64| current.max(index)));
            }
        }
    }
    (
        id.unwrap_or_else(|| stamp.response_id.clone()),
        streamed_model.unwrap_or_else(|| model.to_owned()),
        highest.map_or(0, |index| index.saturating_add(1)),
    )
}

fn continuation_events(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    events: Option<&[Value]>,
    stamp: &BlockStamp,
) -> Vec<Value> {
    let (id, model, index) = continuation_identity(model, events, stamp);
    let item = blocked_output_item(message, stamp);
    let part = json!({"type":"output_text","text":message,"annotations":[]});
    let mut result = events.map_or_else(Vec::new, super::open_item::open_item_closing_events);
    result.extend([
        json!({"type":"response.output_item.added","output_index":index,"item":item}),
        json!({"type":"response.content_part.added","item_id":stamp.message_id,"output_index":index,"content_index":0,"part":part}),
        json!({"type":"response.output_text.delta","item_id":stamp.message_id,"output_index":index,"content_index":0,"delta":message}),
        json!({"type":"response.output_text.done","item_id":stamp.message_id,"output_index":index,"content_index":0,"text":message}),
        json!({"type":"response.content_part.done","item_id":stamp.message_id,"output_index":index,"content_index":0,"part":{"type":"output_text","text":message,"annotations":[],"logprobs":null}}),
        json!({"type":"response.output_item.done","output_index":index,"item":item}),
    ]);
    let usage = serde_json::to_value(blocked_responses_stream_usage(&original(original_response)))
        .unwrap_or_else(|_| json!({}));
    result.push(json!({"type":"response.completed","response":response(&id,&model,stamp.created_at,vec![item],usage)}));
    result
}

fn frame(value: &Value) -> Result<Vec<u8>, BlockEventError> {
    Ok(format!("data: {}\n\n", serde_json::to_string(value)?).into_bytes())
}

pub fn build_block_sse_chunks(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    stream_started: bool,
    responses_so_far: Option<&[Value]>,
    stamp: &BlockStamp,
) -> Result<Vec<Vec<u8>>, BlockEventError> {
    if stream_started {
        return continuation_events(message, model, original_response, responses_so_far, stamp)
            .iter()
            .map(frame)
            .collect();
    }
    let response_value = build_blocked_response(message, model, original_response, stamp);
    let mut response: ResponsesApiResponse = serde_json::from_value(response_value)?;
    let chunk_size = i64::try_from(message.chars().count())
        .unwrap_or(i64::MAX)
        .max(1);
    let events = build_synthetic_response_events(&mut response, Some(&NoCost), chunk_size)?;
    events
        .iter()
        .map(|event| frame(&serde_json::to_value(event)?))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ErrorEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub error: ErrorDetail,
    pub sequence_number: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ErrorDetail {
    #[serde(rename = "type")]
    pub kind: String,
    pub code: String,
    pub message: String,
    pub param: Option<String>,
}

pub fn next_stream_sequence_number(events: Option<&[Value]>) -> i64 {
    events
        .into_iter()
        .flatten()
        .rev()
        .find_map(|event| event.get("sequence_number").and_then(Value::as_i64))
        .map_or(0, |number| number.saturating_add(1))
}

pub fn build_stream_error_items(
    status_code: u16,
    message: &str,
    events: Option<&[Value]>,
) -> Vec<ErrorEvent> {
    vec![ErrorEvent {
        kind: "error".into(),
        error: ErrorDetail {
            kind: "guardrail_error".into(),
            code: status_code.to_string(),
            message: message.into(),
            param: None,
        },
        sequence_number: next_stream_sequence_number(events),
    }]
}

#[cfg(test)]
mod tests;
