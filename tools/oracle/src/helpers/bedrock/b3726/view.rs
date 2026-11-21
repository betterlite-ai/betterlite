use std::collections::BTreeMap;

use betterlite_bedrock::guardrail_hooks::chunks::StreamChunk;
use betterlite_bedrock::guardrail_hooks::errors::HookFailure;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::{Value, json};

use super::assembly::encoded;

pub(super) fn outcome(result: Result<(), HookFailure>) -> Value {
    match result {
        Ok(()) => json!({"kind": "allowed"}),
        Err(HookFailure::Rejected { status, detail }) => json!({
            "kind": "rejected",
            "status_code": status,
            "detail": encoded(&detail),
        }),
        Err(HookFailure::ModifyResponse(failure)) => json!({
            "kind": "modify",
            "message": failure.message,
            "model": failure.model,
        }),
        Err(HookFailure::Failed { message }) => json!({"kind": "failed", "message": message}),
    }
}

pub(super) fn chunks(items: &[StreamChunk]) -> Value {
    Value::Array(items.iter().map(chunk).collect())
}

fn chunk(item: &StreamChunk) -> Value {
    match item {
        StreamChunk::Json(value) => chat(value),
        StreamChunk::Bytes(bytes) => frames(std::str::from_utf8(bytes).unwrap_or_default()),
    }
}

fn chat(value: &GuardrailValue) -> Value {
    let GuardrailValue::Object(fields) = value else {
        return json!({"model": Value::Null, "choices": [], "usage": Value::Null});
    };
    let choices = if let Some(GuardrailValue::Array(items)) = fields.get("choices") {
        items.iter().map(choice).collect()
    } else {
        Vec::new()
    };
    let usage = if let Some(GuardrailValue::Object(usage)) = fields.get("usage") {
        json!({
            "prompt_tokens": usage.get("prompt_tokens").map_or(Value::Null, encoded),
            "completion_tokens": usage.get("completion_tokens").map_or(Value::Null, encoded),
            "total_tokens": usage.get("total_tokens").map_or(Value::Null, encoded),
        })
    } else {
        Value::Null
    };
    json!({
        "model": fields.get("model").map_or(Value::Null, encoded),
        "choices": choices,
        "usage": usage,
    })
}

fn choice(item: &GuardrailValue) -> Value {
    let GuardrailValue::Object(fields) = item else {
        return Value::Null;
    };
    let delta = if let Some(GuardrailValue::Object(delta)) = fields.get("delta") {
        json!({
            "role": delta.get("role").map_or(Value::Null, encoded),
            "content": delta.get("content").map_or(Value::Null, encoded),
        })
    } else {
        Value::Null
    };
    json!({
        "index": fields.get("index").map_or(Value::Null, encoded),
        "delta": delta,
        "finish_reason": fields.get("finish_reason").map_or(Value::Null, encoded),
    })
}

fn frames(text: &str) -> Value {
    let mut parsed = Vec::new();
    for block in text.split("\n\n") {
        let Some(payload) = block.lines().find_map(|line| line.strip_prefix("data:")) else {
            continue;
        };
        let Ok(GuardrailValue::Object(frame)) = serde_json::from_str(payload.trim()) else {
            return json!({"raw": text});
        };
        parsed.push(frame_view(&frame));
    }
    if parsed.is_empty() {
        return json!({"raw": text});
    }
    json!({"frames": parsed})
}

fn frame_view(frame: &BTreeMap<String, GuardrailValue>) -> Value {
    let message = object(frame, "message");
    let delta = object(frame, "delta");
    json!({
        "type": text_in(Some(frame), "type"),
        "index": number_in(Some(frame), "index"),
        "delta_type": text_in(delta, "type"),
        "text": text_in(delta, "text"),
        "stop_reason": text_in(delta, "stop_reason"),
        "block_type": text_in(object(frame, "content_block"), "type"),
        "role": text_in(message, "role"),
        "model": text_in(message, "model"),
        "message_input_tokens": number_in(message.and_then(|fields| object(fields, "usage")), "input_tokens"),
        "message_output_tokens": number_in(message.and_then(|fields| object(fields, "usage")), "output_tokens"),
        "input_tokens": number_in(object(frame, "usage"), "input_tokens"),
        "output_tokens": number_in(object(frame, "usage"), "output_tokens"),
        "error_type": text_in(object(frame, "error"), "type"),
        "error_message": text_in(object(frame, "error"), "message"),
    })
}

fn object<'a>(
    fields: &'a BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<&'a BTreeMap<String, GuardrailValue>> {
    match fields.get(key) {
        Some(GuardrailValue::Object(inner)) => Some(inner),
        None
        | Some(GuardrailValue::Null)
        | Some(GuardrailValue::Boolean(_))
        | Some(GuardrailValue::SignedInteger(_))
        | Some(GuardrailValue::UnsignedInteger(_))
        | Some(GuardrailValue::Decimal(_))
        | Some(GuardrailValue::Text(_))
        | Some(GuardrailValue::Array(_)) => None,
    }
}

fn text_in(fields: Option<&BTreeMap<String, GuardrailValue>>, key: &str) -> Value {
    if let Some(GuardrailValue::Text(text)) = fields.and_then(|fields| fields.get(key)) {
        return Value::String(text.clone());
    }
    Value::Null
}

fn number_in(fields: Option<&BTreeMap<String, GuardrailValue>>, key: &str) -> Value {
    match fields.and_then(|fields| fields.get(key)) {
        Some(GuardrailValue::SignedInteger(count)) => json!(count),
        Some(GuardrailValue::UnsignedInteger(count)) => json!(count),
        None
        | Some(GuardrailValue::Null)
        | Some(GuardrailValue::Boolean(_))
        | Some(GuardrailValue::Decimal(_))
        | Some(GuardrailValue::Text(_))
        | Some(GuardrailValue::Array(_))
        | Some(GuardrailValue::Object(_)) => Value::Null,
    }
}
