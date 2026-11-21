use std::collections::BTreeMap;

use betterlite_bedrock::guardrail_hooks::chunks::StreamChunk;
use betterlite_bedrock::guardrail_hooks::errors::{BlockedChat, HookFailure};
use betterlite_bedrock::guardrail_hooks::response::chat_chunk;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::{Value, json};

pub(super) fn assemble(chunks: &[StreamChunk], raw_sse: bool) -> Option<GuardrailValue> {
    if raw_sse {
        return assemble_sse(chunks);
    }
    assemble_chat(chunks)
}

pub(super) fn encoded(value: &GuardrailValue) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn from_json(value: Value) -> Option<GuardrailValue> {
    serde_json::from_value(value).ok()
}

fn field(fields: &BTreeMap<String, GuardrailValue>, key: &str) -> Option<Value> {
    let value = fields.get(key)?;
    if matches!(value, GuardrailValue::Null) {
        return None;
    }
    Some(encoded(value))
}

fn text_of(fields: &BTreeMap<String, GuardrailValue>, key: &str) -> Option<String> {
    if let Some(GuardrailValue::Text(text)) = fields.get(key) {
        return Some(text.clone());
    }
    None
}

fn integer_of(fields: &BTreeMap<String, GuardrailValue>, key: &str) -> i64 {
    match fields.get(key) {
        Some(GuardrailValue::SignedInteger(count)) => *count,
        Some(GuardrailValue::UnsignedInteger(count)) => i64::try_from(*count).unwrap_or(i64::MAX),
        None
        | Some(GuardrailValue::Null)
        | Some(GuardrailValue::Boolean(_))
        | Some(GuardrailValue::Decimal(_))
        | Some(GuardrailValue::Text(_))
        | Some(GuardrailValue::Array(_))
        | Some(GuardrailValue::Object(_)) => 0,
    }
}

// LiteLLM's stream_chunk_builder over the plain chat chunks the cases replay: deltas merge in order, the last usage wins.
fn assemble_chat(chunks: &[StreamChunk]) -> Option<GuardrailValue> {
    let mut text = String::new();
    let mut finish = Value::Null;
    let mut model = Value::Null;
    let mut usage = Value::Null;
    for chunk in chunks {
        let StreamChunk::Json(GuardrailValue::Object(fields)) = chunk else {
            return None;
        };
        if let Some(value) = field(fields, "model") {
            model = value;
        }
        if let Some(value) = field(fields, "usage") {
            usage = value;
        }
        let Some(GuardrailValue::Array(choices)) = fields.get("choices") else {
            continue;
        };
        for choice in choices {
            let GuardrailValue::Object(choice_fields) = choice else {
                return None;
            };
            if let Some(GuardrailValue::Object(delta)) = choice_fields.get("delta")
                && let Some(GuardrailValue::Text(part)) = delta.get("content")
            {
                text.push_str(part);
            }
            if let Some(reason) = field(choice_fields, "finish_reason") {
                finish = reason;
            }
        }
    }
    from_json(json!({
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": finish,
        }],
        "usage": usage,
    }))
}

// LiteLLM's assemble_anthropic_sse_stream with restore_identity: message_start names the message, text deltas fill it.
fn assemble_sse(chunks: &[StreamChunk]) -> Option<GuardrailValue> {
    let mut id = String::new();
    let mut model = String::new();
    let mut text = String::new();
    let mut input_tokens = 0_i64;
    let mut output_tokens = 0_i64;
    let mut started = false;
    for chunk in chunks {
        let StreamChunk::Bytes(bytes) = chunk else {
            return None;
        };
        let frame = std::str::from_utf8(bytes).ok()?;
        let payload = frame
            .lines()
            .find_map(|line| line.strip_prefix("data:"))?
            .trim();
        let GuardrailValue::Object(event) = serde_json::from_str(payload).ok()? else {
            return None;
        };
        let kind = text_of(&event, "type").unwrap_or_default();
        match kind.as_str() {
            "message_start" => {
                started = true;
                let Some(GuardrailValue::Object(message)) = event.get("message") else {
                    continue;
                };
                if let Some(value) = text_of(message, "id") {
                    id = value;
                }
                if let Some(value) = text_of(message, "model") {
                    model = value;
                }
                if let Some(GuardrailValue::Object(usage)) = message.get("usage") {
                    input_tokens = integer_of(usage, "input_tokens");
                }
            }
            "content_block_delta" => {
                if let Some(GuardrailValue::Object(delta)) = event.get("delta")
                    && let Some(GuardrailValue::Text(part)) = delta.get("text")
                {
                    text.push_str(part);
                }
            }
            "message_delta" => {
                if let Some(GuardrailValue::Object(usage)) = event.get("usage") {
                    output_tokens = integer_of(usage, "output_tokens");
                }
            }
            _ => {}
        }
    }
    if !started {
        return None;
    }
    from_json(json!({
        "id": id,
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens.saturating_add(output_tokens),
        },
    }))
}

fn block_failure() -> HookFailure {
    HookFailure::Failed {
        message: "the block reply could not be replayed as a chunk".to_owned(),
    }
}

// MockResponseIterator over the replacement response: one chunk carrying the block message and the usage it replaced.
pub(super) fn block_chat_chunks(block: &BlockedChat) -> Result<Vec<StreamChunk>, HookFailure> {
    let value = from_json(json!({
        "model": block.model,
        "usage": block.usage,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": block.message},
            "finish_reason": "content_filter",
        }],
    }))
    .ok_or_else(block_failure)?;
    chat_chunk(&value)
        .map(|chunk| vec![chunk])
        .ok_or_else(block_failure)
}
