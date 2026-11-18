#![expect(
    clippy::disallowed_types,
    reason = "Anthropic event payloads preserve provider JSON fields"
)]

use betterlite_core::api::messages_fake_stream::MessagesFakeStream;
use betterlite_guardrails::anthropic_sse::error_frames::anthropic_sse_error_frames;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::sse_events::{SseChunk, content_block_state};

#[derive(Debug, thiserror::Error)]
pub enum AnthropicBlockError {
    #[error(transparent)]
    FakeStream(#[from] betterlite_core::api::messages_fake_stream::MessagesFakeStreamError),
    #[error("Anthropic stream event serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    ErrorFrames(#[from] betterlite_guardrails::anthropic_sse::error::AnthropicSseError),
}

fn original(
    value: Option<&Value>,
) -> betterlite_guardrails::guardrail_message_scope::blocked_usage::OriginalResponse {
    let Some(fields) = value.and_then(Value::as_object) else {
        return betterlite_guardrails::guardrail_message_scope::blocked_usage::OriginalResponse::Absent;
    };
    let object = fields
        .iter()
        .map(|(key, value)| (key.clone(), guardrail_value(value)))
        .collect();
    betterlite_guardrails::guardrail_message_scope::blocked_usage::OriginalResponse::Response(
        object,
    )
}

fn guardrail_value(
    value: &Value,
) -> betterlite_config::sections::guardrail_identity::item::GuardrailValue {
    use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
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
        Value::Array(values) => GuardrailValue::Array(values.iter().map(guardrail_value).collect()),
        Value::Object(values) => GuardrailValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), guardrail_value(value)))
                .collect(),
        ),
    }
}

pub fn build_block_sse_chunks(
    message: &str,
    model: &str,
    original_response: Option<&Value>,
    stream_started: bool,
    responses_so_far: Option<&[SseChunk]>,
    message_id: &str,
) -> Result<Vec<Vec<u8>>, AnthropicBlockError> {
    let usage = serde_json::to_value(
        betterlite_guardrails::guardrail_message_scope::blocked_usage::blocked_response_usage(
            &original(original_response),
        ),
    )?;
    if !stream_started {
        let response = json!({"id":message_id,"type":"message","role":"assistant","content":[{"type":"text","text":message}],"model":model,"stop_reason":"end_turn","usage":usage});
        let response: Json = serde_json::from_value(response)?;
        return Ok(MessagesFakeStream::new(&response)?.collect());
    }
    let (open, highest) = content_block_state(responses_so_far.unwrap_or_default());
    let new_index = highest.map_or(0, |index| index.saturating_add(1));
    let mut events = Vec::<(&str, Value)>::new();
    if let Some(index) = open {
        events.push((
            "content_block_stop",
            json!({"type":"content_block_stop","index":index}),
        ));
    }
    events.extend([
        ("content_block_start",json!({"type":"content_block_start","index":new_index,"content_block":{"type":"text","text":""}})),
        ("content_block_delta",json!({"type":"content_block_delta","index":new_index,"delta":{"type":"text_delta","text":message}})),
        ("content_block_stop",json!({"type":"content_block_stop","index":new_index})),
        ("message_delta",json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":usage.get("output_tokens").cloned().unwrap_or(json!(0))}})),
        ("message_stop",json!({"type":"message_stop"})),
    ]);
    Ok(events
        .into_iter()
        .map(|(kind, event)| {
            let data = python_json(&event);
            format!("event: {kind}\ndata: {data}\n\n").into_bytes()
        })
        .collect())
}

fn python_json(value: &Value) -> String {
    betterlite_logging::python_json::dumps(&decision_json(value, ""))
}

fn decision_json(value: &Value, parent: &str) -> Json {
    match value {
        Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort_by_key(|key| ordered_key(parent, key));
            let mut ordered = IndexMap::with_capacity(keys.len());
            for key in keys {
                ordered.insert(key.clone(), decision_json(&fields[key], key));
            }
            Json::Object(ordered)
        }
        Value::Array(values) => Json::List(
            values
                .iter()
                .map(|value| decision_json(value, parent))
                .collect(),
        ),
        Value::Null => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Number(value) => Json::Number(value.clone()),
        Value::String(value) => Json::Text(value.clone()),
    }
}

fn ordered_key(parent: &str, key: &str) -> usize {
    match (parent, key) {
        ("", "type") => 0,
        ("", "index") => 1,
        ("", "content_block" | "delta") => 2,
        ("", "usage") => 3,
        ("content_block", "type") => 0,
        ("content_block", "text") => 1,
        ("delta", "stop_reason") => 0,
        ("delta", "stop_sequence") => 1,
        ("usage", "output_tokens") => 0,
        _ => 10,
    }
}

pub fn build_stream_error_items(message: &str) -> Result<Vec<Vec<u8>>, AnthropicBlockError> {
    Ok(anthropic_sse_error_frames(message)?)
}

#[cfg(test)]
mod tests;
