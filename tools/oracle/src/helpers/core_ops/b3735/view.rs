use std::str::from_utf8;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::model_armor::streams::chunk::{Chunk, field, text_field};
use betterlite_core::guardrails::model_armor::streams::wire::OutputParts;
use betterlite_core::guardrails::streaming_rewrite::shapes::RequestData;
use serde_json::{Map, Value, json};

use super::{APPLIED_KEY, METADATA_KEY};

const RESPONSE_KEY: &str = "_model_armor_response";
const STATUS_KEY: &str = "_model_armor_status";

pub(super) fn item(chunk: &Chunk) -> Value {
    match chunk {
        Chunk::Json(payload) => payload_view(payload),
        Chunk::Bytes(bytes) => bytes_view(bytes),
    }
}

fn payload_view(payload: &GuardrailValue) -> Value {
    match text_field(field(payload, "type")) {
        Some("error") => {
            let error = field(payload, "error");
            json!({
                "kind": "error_item",
                "error_type": plain(error.and_then(|value| field(value, "type"))),
                "code": plain(error.and_then(|value| field(value, "code"))),
                "message": plain(error.and_then(|value| field(value, "message"))),
                "param": plain(error.and_then(|value| field(value, "param"))),
            })
        }
        Some(kind) if kind.starts_with("response.") => json!({
            "kind": "event",
            "type": plain(field(payload, "type")),
            "delta": plain(field(payload, "delta")),
            "item_id": plain(field(payload, "item_id")),
            "output_index": plain(field(payload, "output_index")),
            "content_index": plain(field(payload, "content_index")),
            "summary_index": plain(field(payload, "summary_index")),
        }),
        Some(_) | None => json!({
            "kind": "chat",
            "id": plain(field(payload, "id")),
            "object": plain(field(payload, "object")),
            "model": plain(field(payload, "model")),
            "created": plain(field(payload, "created")),
            "choices": choices_view(payload),
        }),
    }
}

fn choices_view(payload: &GuardrailValue) -> Value {
    let Some(GuardrailValue::Array(items)) = field(payload, "choices") else {
        return Value::Array(Vec::new());
    };
    let mut views = Vec::with_capacity(items.len());
    for choice in items {
        let delta = field(choice, "delta");
        views.push(json!({
            "index": plain(field(choice, "index")),
            "finish_reason": plain(field(choice, "finish_reason")),
            "role": plain(delta.and_then(|value| field(value, "role"))),
            "content": plain(delta.and_then(|value| field(value, "content"))),
        }));
    }
    Value::Array(views)
}

fn bytes_view(bytes: &[u8]) -> Value {
    let Ok(frame) = from_utf8(bytes) else {
        return json!({"kind": "sse", "event": null, "data": null});
    };
    let mut event = None;
    let mut data = Value::Null;
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_owned());
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data = serde_json::from_str(rest.trim()).unwrap_or(Value::Null);
        }
    }
    json!({"kind": "sse", "event": event, "data": frame_body(&data)})
}

fn frame_body(data: &Value) -> Value {
    let Some(fields) = data.as_object() else {
        return Value::Object(Map::new());
    };
    json!({
        "type": fields.get("type").cloned().unwrap_or(Value::Null),
        "message_id": nested(fields, "message", "id"),
        "delta_type": nested(fields, "delta", "type"),
        "text": nested(fields, "delta", "text"),
        "stop_reason": nested(fields, "delta", "stop_reason"),
        "block_type": nested(fields, "content_block", "type"),
        "error_type": nested(fields, "error", "type"),
        "error_message": nested(fields, "error", "message"),
    })
}

fn nested(fields: &Map<String, Value>, outer: &str, inner: &str) -> Value {
    fields
        .get(outer)
        .and_then(Value::as_object)
        .and_then(|object| object.get(inner))
        .cloned()
        .unwrap_or(Value::Null)
}

pub(super) fn output_parts(item: &GuardrailValue) -> OutputParts {
    let mut parts = OutputParts::default();
    let Some(GuardrailValue::Array(content)) = field(item, "content") else {
        return parts;
    };
    for entry in content {
        if let Some(GuardrailValue::Text(text)) = field(entry, "text") {
            parts.texts.push(text.clone());
        }
    }
    parts
}

pub(super) fn metadata(data: &RequestData) -> Value {
    let bucket = match data.get(METADATA_KEY) {
        Some(GuardrailValue::Object(fields)) => Some(fields),
        _ => None,
    };
    json!({
        "response": plain(bucket.and_then(|fields| fields.get(RESPONSE_KEY))),
        "status": plain(bucket.and_then(|fields| fields.get(STATUS_KEY))),
        "applied": plain(bucket.and_then(|fields| fields.get(APPLIED_KEY))),
    })
}

fn plain(value: Option<&GuardrailValue>) -> Value {
    value.map_or(Value::Null, |found| {
        serde_json::to_value(found).unwrap_or(Value::Null)
    })
}
