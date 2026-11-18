#![expect(
    clippy::disallowed_types,
    clippy::too_many_arguments,
    reason = "Responses stream data is an open wire boundary, and build_block_sse_chunks keeps LiteLLM's argument list"
)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_synthetic_events::{
    builder::build_synthetic_response_events,
    stamping::{CostTarget, ResponsesCostCalculator},
};
use betterlite_guardrails::guardrail_message_scope::{StreamItem, fingerprint::stream_item_field};
use betterlite_openai::api::responses_event_records::event_types::{
    ErrorEvent, ErrorEventError, Extra,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::convert::Infallible;

use guardrail_json::{to_guardrail, usage_value};

use super::open_item::open_item_closing_events;

pub mod guardrail_json;

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
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

fn stream_string_field(event: &Value, name: &str) -> Option<String> {
    let item = StreamItem::Json(to_guardrail(event));
    match stream_item_field(&item, name)? {
        GuardrailValue::Text(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn stream_index(event: &Value) -> Option<i64> {
    let item = StreamItem::Json(to_guardrail(event));
    match stream_item_field(&item, "output_index")? {
        GuardrailValue::SignedInteger(index) => Some(*index),
        GuardrailValue::UnsignedInteger(index) => i64::try_from(*index).ok(),
        _ => None,
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
    json!({
        "type": "message",
        "id": stamp.message_id,
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": message, "annotations": []}],
    })
}

fn blocked_response(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    stamp: &BlockStamp,
    response_id: &str,
    output_item: Option<&Value>,
) -> Value {
    let item = output_item.map_or_else(|| blocked_output_item(message, stamp), Value::clone);
    json!({
        "id": response_id,
        "object": "response",
        "created_at": stamp.created_at,
        "model": model,
        "output": [item],
        "status": "completed",
        "usage": usage_value(original_response),
    })
}

pub fn build_blocked_response(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    stamp: &BlockStamp,
) -> Value {
    blocked_response(
        message,
        model,
        original_response,
        stamp,
        &stamp.response_id,
        None,
    )
}

pub fn continuation_identity(
    model: &str,
    responses_so_far: Option<&[Value]>,
    stamp: &BlockStamp,
) -> (String, String, i64) {
    let mut response_id = None;
    let mut streamed_model = None;
    let mut highest = None;
    for event in responses_so_far.into_iter().flatten() {
        if let Some(response) = event.get("response") {
            if response_id.is_none() {
                response_id = stream_string_field(response, "id");
            }
            if streamed_model.is_none() {
                streamed_model = stream_string_field(response, "model");
            }
        }
        if let Some(index) = stream_index(event) {
            highest = Some(highest.map_or(index, |current: i64| current.max(index)));
        }
    }
    (
        response_id.unwrap_or_else(|| stamp.response_id.clone()),
        streamed_model.unwrap_or_else(|| model.to_owned()),
        highest.map_or(0, |index| index.saturating_add(1)),
    )
}

fn block_item_events(message: &str, index: i64, item: &Value, stamp: &BlockStamp) -> Vec<Value> {
    let part = json!({"type": "output_text", "text": message, "annotations": []});
    vec![
        json!({"type": "response.output_item.added", "output_index": index, "item": item}),
        json!({
            "type": "response.content_part.added",
            "item_id": stamp.message_id,
            "output_index": index,
            "content_index": 0,
            "part": part,
        }),
        json!({
            "type": "response.output_text.delta",
            "item_id": stamp.message_id,
            "output_index": index,
            "content_index": 0,
            "delta": message,
        }),
        json!({
            "type": "response.output_text.done",
            "item_id": stamp.message_id,
            "output_index": index,
            "content_index": 0,
            "text": message,
        }),
        json!({
            "type": "response.content_part.done",
            "item_id": stamp.message_id,
            "output_index": index,
            "content_index": 0,
            "part": part,
        }),
        json!({"type": "response.output_item.done", "output_index": index, "item": item}),
    ]
}

fn block_continuation_events(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    responses_so_far: Option<&[Value]>,
    stamp: &BlockStamp,
) -> Vec<Value> {
    let (response_id, model, index) = continuation_identity(model, responses_so_far, stamp);
    let item = blocked_output_item(message, stamp);
    let mut events = responses_so_far.map_or_else(Vec::new, open_item_closing_events);
    events.extend(block_item_events(message, index, &item, stamp));
    let response = blocked_response(
        message,
        model,
        original_response,
        stamp,
        &response_id,
        Some(&item),
    );
    events.push(json!({"type": "response.completed", "response": response}));
    events
}

fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(name, value)| (name, without_nulls(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_nulls).collect()),
        value => value,
    }
}

fn frame(value: &Value) -> Result<Vec<u8>, BlockEventError> {
    let payload = serde_json::to_string(&without_nulls(value.clone()))?;
    Ok(format!("data: {payload}\n\n").into_bytes())
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
        return block_continuation_events(
            message,
            model,
            original_response,
            responses_so_far,
            stamp,
        )
        .iter()
        .map(frame)
        .collect();
    }
    let mut response: ResponsesApiResponse = serde_json::from_value(build_blocked_response(
        message,
        model,
        original_response,
        stamp,
    ))?;
    let chunk_size = i64::try_from(message.chars().count())
        .unwrap_or(i64::MAX)
        .max(1);
    build_synthetic_response_events(&mut response, Some(&NoCost), chunk_size)?
        .iter()
        .map(|event| frame(&serde_json::to_value(event)?))
        .collect()
}

pub fn next_stream_sequence_number(responses_so_far: Option<&[Value]>) -> i64 {
    responses_so_far
        .into_iter()
        .flatten()
        .rev()
        .find_map(|event| event.get("sequence_number").and_then(Value::as_i64))
        .map_or(0, |number| number.saturating_add(1))
}

pub fn build_stream_error_items(
    status_code: u16,
    message: &str,
    responses_so_far: Option<&[Value]>,
) -> Vec<ErrorEvent> {
    vec![ErrorEvent {
        sequence_number: next_stream_sequence_number(responses_so_far),
        error: ErrorEventError {
            r#type: "guardrail_error".to_owned(),
            code: status_code.to_string(),
            message: message.to_owned(),
            param: None,
            extra: Extra::new(),
        },
        extra: Extra::new(),
    }]
}

#[cfg(test)]
mod tests;
