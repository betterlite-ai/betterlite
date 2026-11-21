use betterlite_bedrock::guardrail_hooks::chunks::StreamChunk;
use betterlite_bedrock::guardrail_hooks::errors::HookFailure;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::{Value, json};

use super::assembly::{encoded, merge_choices};

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
    let mut rendered = Vec::new();
    for item in items {
        rendered.push(chunk(item));
    }
    Value::Array(rendered)
}

fn chunk(item: &StreamChunk) -> Value {
    match item {
        StreamChunk::Json(value) => chat(value),
        StreamChunk::Bytes(bytes) => frames(std::str::from_utf8(bytes).unwrap_or_default()),
    }
}

// The chunk fields a caller can see change: the model, the assistant text, the usage and the finish reason.
fn chat(value: &GuardrailValue) -> Value {
    let GuardrailValue::Object(fields) = value else {
        return Value::Null;
    };
    let mut text = String::new();
    let mut finish = Value::Null;
    if let Some(GuardrailValue::Array(choices)) = fields.get("choices")
        && merge_choices(choices, &mut text, &mut finish).is_none()
    {
        return Value::Null;
    }
    json!({
        "model": fields.get("model").map_or(Value::Null, encoded),
        "text": text,
        "usage": usage_view(fields.get("usage")),
        "finish_reason": finish,
    })
}

fn usage_view(value: Option<&GuardrailValue>) -> Value {
    let Some(GuardrailValue::Object(usage)) = value else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": usage.get("prompt_tokens").map_or(Value::Null, encoded),
        "completion_tokens": usage.get("completion_tokens").map_or(Value::Null, encoded),
        "total_tokens": usage.get("total_tokens").map_or(Value::Null, encoded),
    })
}

fn frames(text: &str) -> Value {
    let mut parsed = Vec::new();
    for block in text.split("\n\n") {
        let Some(payload) = block.lines().find_map(|line| line.strip_prefix("data:")) else {
            continue;
        };
        let Ok(frame) = serde_json::from_str::<Value>(payload.trim()) else {
            return json!({"raw": text});
        };
        parsed.push(frame);
    }
    if parsed.is_empty() {
        return json!({"raw": text});
    }
    json!({"frames": parsed})
}
