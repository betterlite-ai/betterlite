#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::too_many_arguments,
    reason = "Responses stream data is an open wire boundary, and build_block_sse_chunks keeps LiteLLM's argument list"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        reason = "serde_json fixtures compare JSON numbers and construct provider objects"
    )
)]

use crate::stream_chunks::{ChunkKind, StreamChunk, serialize::serialize_chunk};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::response_synthetic_events::{
    builder,
    error::SyntheticEventsError,
    stamping::{CostTarget, ResponsesCostCalculator},
};
use betterlite_guardrails::guardrail_message_scope::{StreamItem, fingerprint::stream_item_field};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::responses_event_records::event_types::{
    ErrorEvent, ErrorEventError, Extra,
};
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_guardrails::guardrail_message_scope::blocked_usage::blocked_responses_stream_usage;
use guardrail_json::{blocked_original_response, to_guardrail};

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
    Synthetic(#[from] SyntheticEventsError),
}

fn stream_string_field(event: &Value, name: &str) -> Option<String> {
    let item = StreamItem::Json(to_guardrail(event));
    match stream_item_field(&item, name)? {
        GuardrailValue::Text(text) if !text.is_empty() => Some(text.clone()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => None,
    }
}

fn stream_index(event: &Value) -> Option<i64> {
    let item = StreamItem::Json(to_guardrail(event));
    match stream_item_field(&item, "output_index")? {
        GuardrailValue::SignedInteger(index) => Some(*index),
        GuardrailValue::UnsignedInteger(index) => Some(i64::try_from(*index).unwrap_or(i64::MAX)),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => None,
    }
}

struct NoCost;

impl ResponsesCostCalculator for NoCost {
    type Error = Infallible;

    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error> {
        Ok(None)
    }
}

#[must_use]
pub fn blocked_output_item(message: &str, stamp: &BlockStamp) -> Value {
    json!({
        "type": "message",
        "id": stamp.message_id,
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": message, "annotations": []}],
    })
}

#[must_use]
pub fn build_blocked_response(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    stamp: &BlockStamp,
) -> Value {
    let original = blocked_original_response(original_response);
    let usage = blocked_responses_stream_usage(&original);
    json!({
        "id": stamp.response_id,
        "object": "response",
        "created_at": stamp.created_at,
        "model": model,
        "output": [blocked_output_item(message, stamp)],
        "status": "completed",
        "usage": usage,
    })
}

#[must_use]
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
            "content_index": 0_i64,
            "part": part,
        }),
        json!({
            "type": "response.output_text.delta",
            "item_id": stamp.message_id,
            "output_index": index,
            "content_index": 0_i64,
            "delta": message,
        }),
        json!({
            "type": "response.output_text.done",
            "item_id": stamp.message_id,
            "output_index": index,
            "content_index": 0_i64,
            "text": message,
        }),
        json!({
            "type": "response.content_part.done",
            "item_id": stamp.message_id,
            "output_index": index,
            "content_index": 0_i64,
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
    let (response_id, streamed_model, index) =
        continuation_identity(model, responses_so_far, stamp);
    let item = blocked_output_item(message, stamp);
    let mut events = responses_so_far.map_or_else(Vec::new, open_item_closing_events);
    events.extend(block_item_events(message, index, &item, stamp));
    let continued_stamp = BlockStamp {
        response_id,
        message_id: stamp.message_id.clone(),
        created_at: stamp.created_at,
    };
    let response = build_blocked_response(
        message,
        &streamed_model,
        original_response,
        &continued_stamp,
    );
    events.push(json!({"type": "response.completed", "response": response}));
    events
}

fn frame(value: &Value) -> Result<Vec<u8>, BlockEventError> {
    let chunk = StreamChunk {
        body: serde_json::from_value(value.clone())?,
        kind: ChunkKind::Event,
        hidden: JsonObject::new(),
    };
    let payload = serialize_chunk(&chunk);
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
    let item: BTreeMap<String, Json> = serde_json::from_value(blocked_output_item(message, stamp))?;
    // the synthetic events dump the item for the wire, and a typed parse would add fields LiteLLM's generic item omits
    response.output = vec![ResponseOutputItem::Unknown(item)];
    let chunk_size = i64::try_from(message.chars().count())
        .unwrap_or(i64::MAX)
        .max(1);
    // ast-grep-ignore: no-long-names
    builder::build_synthetic_response_events(&mut response, Some(&NoCost), chunk_size)?
        .iter()
        .map(|event| frame(&serde_json::to_value(event)?))
        .collect()
}

#[must_use]
pub fn next_stream_sequence_number(responses_so_far: Option<&[Value]>) -> i64 {
    responses_so_far
        .into_iter()
        .flatten()
        .rev()
        .find_map(|event| event.get("sequence_number").and_then(Value::as_i64))
        .map_or(0, |number| number.saturating_add(1))
}

#[must_use]
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

mod tests;
