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
use betterlite_guardrails::guardrail_message_scope::{StreamItem, fingerprint::stream_item_field};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Map, Value, json};
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

fn stream_string_field(event: &Value, field: &str) -> Option<String> {
    let item = StreamItem::Json(to_guardrail(event));
    match stream_item_field(&item, field)? {
        GuardrailValue::Text(value) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn stream_index(event: &Value) -> Option<i64> {
    let item = StreamItem::Json(to_guardrail(event));
    match stream_item_field(&item, "output_index")? {
        GuardrailValue::SignedInteger(value) => Some(*value),
        GuardrailValue::UnsignedInteger(value) => i64::try_from(*value).ok(),
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
                    id = stream_string_field(response, "id");
                }
                if streamed_model.is_none() {
                    streamed_model = stream_string_field(response, "model");
                }
            }
            if let Some(index) = stream_index(event) {
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
    let mut result = events.map_or_else(Vec::new, open_item_closing_events);
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

use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct OpenItemState {
    pub item_id: String,
    pub item_type: String,
    pub role: String,
    pub output_index: i64,
    pub content_index: i64,
    pub text: String,
    pub part_open: bool,
    pub payload: Value,
}

pub fn incomplete_item_fields(payload: &Value) -> Map<String, Value> {
    payload.as_object().cloned().unwrap_or_default()
}

pub fn open_item_state(events: &[Value]) -> Option<OpenItemState> {
    let mut added = Vec::<(i64, Value)>::new();
    let mut done = Vec::<i64>::new();
    for event in events {
        let index = event.get("output_index").and_then(Value::as_i64);
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_item.added") => {
                if let (Some(index), Some(item)) = (index, event.get("item")) {
                    added.push((index, item.clone()));
                }
            }
            Some("response.output_item.done") => {
                if let Some(index) = index {
                    done.push(index);
                }
            }
            _ => {}
        }
    }
    let (output_index, payload) = added
        .into_iter()
        .rev()
        .find(|(index, _)| !done.contains(index))?;
    let item_id = payload.get("id")?.as_str()?.to_owned();
    if item_id.is_empty() {
        return None;
    }
    let item_type = payload
        .get("type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("message")
        .to_owned();
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("assistant")
        .to_owned();
    let mut parts = Vec::<i64>::new();
    let mut closed = Vec::<i64>::new();
    let mut text = String::new();
    for event in events {
        if event.get("item_id").and_then(Value::as_str) != Some(item_id.as_str()) {
            continue;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("response.content_part.added") => {
                if let Some(index) = event.get("content_index").and_then(Value::as_i64) {
                    parts.push(index);
                }
            }
            Some("response.content_part.done") => {
                if let Some(index) = event.get("content_index").and_then(Value::as_i64) {
                    closed.push(index);
                }
            }
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    text.push_str(delta);
                }
            }
            _ => {}
        }
    }
    let content_index = parts
        .iter()
        .rev()
        .find(|index| !closed.contains(index))
        .copied()
        .unwrap_or(0);
    Some(OpenItemState {
        item_id,
        item_type,
        role,
        output_index,
        content_index,
        text,
        part_open: parts.iter().any(|index| !closed.contains(index)),
        payload,
    })
}

pub fn open_item_closing_events(events: &[Value]) -> Vec<Value> {
    let Some(state) = open_item_state(events) else {
        return Vec::new();
    };
    if state.item_type != "message" {
        let mut item = incomplete_item_fields(&state.payload);
        item.insert("status".into(), json!("incomplete"));
        return vec![
            json!({"type":"response.output_item.done","output_index":state.output_index,"item":item}),
        ];
    }
    let mut result = Vec::new();
    if state.part_open {
        result.push(json!({"type":"response.output_text.done","item_id":state.item_id,"output_index":state.output_index,"content_index":state.content_index,"text":state.text}));
        result.push(json!({"type":"response.content_part.done","item_id":state.item_id,"output_index":state.output_index,"content_index":state.content_index,"part":{"type":"output_text","text":state.text,"annotations":[],"logprobs":null}}));
    }
    result.push(json!({"type":"response.output_item.done","output_index":state.output_index,"item":{"id":state.item_id,"type":"message","status":"completed","role":state.role,"content":[{"type":"output_text","text":state.text,"annotations":[]} ]}}));
    result
}

#[cfg(test)]
mod tests;
