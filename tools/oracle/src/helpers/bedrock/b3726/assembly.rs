use std::collections::BTreeMap;
use std::str::from_utf8;

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
    if let Ok(payload) = serde_json::to_value(value) {
        return payload;
    }
    Value::Null
}

fn from_json(value: Value) -> Option<GuardrailValue> {
    if let Ok(parsed) = serde_json::from_value(value) {
        return Some(parsed);
    }
    None
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
    if let Some(GuardrailValue::SignedInteger(count)) = fields.get(key) {
        return *count;
    }
    if let Some(GuardrailValue::UnsignedInteger(count)) = fields.get(key) {
        return i64::try_from(*count).unwrap_or(i64::MAX);
    }
    0
}

pub(super) fn merge_choices(
    choices: &[GuardrailValue],
    text: &mut String,
    finish: &mut Value,
) -> Option<()> {
    for choice in choices {
        let GuardrailValue::Object(choice_fields) = choice else {
            return None;
        };
        if let Some(GuardrailValue::Object(delta)) = choice_fields.get("delta")
            && let Some(GuardrailValue::Text(part)) = delta.get("content")
        {
            text.push_str(part);
        }
        if let Some(GuardrailValue::Text(part)) = choice_fields.get("text") {
            text.push_str(part);
        }
        if let Some(reason) = field(choice_fields, "finish_reason") {
            *finish = reason;
        }
    }
    Some(())
}

// LiteLLM's stream_chunk_builder over the chunks the cases replay: deltas merge in order, the last usage wins.
fn assemble_chat(chunks: &[StreamChunk]) -> Option<GuardrailValue> {
    let mut text = String::new();
    let mut finish = Value::Null;
    let mut model = Value::Null;
    let mut usage = Value::Null;
    let mut chosen = false;
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
        // A first choice that carries text makes LiteLLM's stream_chunk_builder build a text completion, not a chat ModelResponse.
        if !chosen && !choices.is_empty() {
            chosen = true;
            if matches!(
                choices.first(),
                Some(GuardrailValue::Object(first)) if first.contains_key("text")
            ) {
                return None;
            }
        }
        merge_choices(choices, &mut text, &mut finish)?;
    }
    from_json(json!({
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0_i32,
            "message": {"role": "assistant", "content": text},
            "finish_reason": finish,
        }],
        "usage": usage,
    }))
}

fn frame_event(chunk: &StreamChunk) -> Option<BTreeMap<String, GuardrailValue>> {
    let StreamChunk::Bytes(bytes) = chunk else {
        return None;
    };
    let frame = from_utf8(bytes).ok()?;
    let payload = frame
        .lines()
        .find_map(|line| line.strip_prefix("data:"))?
        .trim();
    let GuardrailValue::Object(event) = serde_json::from_str(payload).ok()? else {
        return None;
    };
    Some(event)
}

// LiteLLM's assemble_anthropic_sse_stream with restore_identity: message_start names the message, text deltas fill it.
#[derive(Default)]
struct SseBuilder {
    id: String,
    model: String,
    text: String,
    input_tokens: i64,
    output_tokens: i64,
    started: bool,
}

impl SseBuilder {
    fn absorb(&mut self, chunk: &StreamChunk) -> Option<()> {
        let event = frame_event(chunk)?;
        let kind = text_of(&event, "type").unwrap_or_default();
        match kind.as_str() {
            "message_start" => self.start(&event),
            "content_block_delta" => self.delta(&event),
            "message_delta" => self.usage(&event),
            _ => {}
        }
        Some(())
    }

    fn start(&mut self, event: &BTreeMap<String, GuardrailValue>) {
        self.started = true;
        let Some(GuardrailValue::Object(message)) = event.get("message") else {
            return;
        };
        if let Some(value) = text_of(message, "id") {
            self.id = value;
        }
        if let Some(value) = text_of(message, "model") {
            self.model = value;
        }
        if let Some(GuardrailValue::Object(usage)) = message.get("usage") {
            self.input_tokens = integer_of(usage, "input_tokens");
        }
    }

    fn delta(&mut self, event: &BTreeMap<String, GuardrailValue>) {
        if let Some(GuardrailValue::Object(delta)) = event.get("delta")
            && let Some(GuardrailValue::Text(part)) = delta.get("text")
        {
            self.text.push_str(part);
        }
    }

    fn usage(&mut self, event: &BTreeMap<String, GuardrailValue>) {
        if let Some(GuardrailValue::Object(usage)) = event.get("usage") {
            self.output_tokens = integer_of(usage, "output_tokens");
        }
    }

    fn finish(self) -> Option<GuardrailValue> {
        if !self.started {
            return None;
        }
        from_json(json!({
            "id": self.id,
            "object": "chat.completion",
            "model": self.model,
            "choices": [{
                "index": 0_i32,
                "message": {"role": "assistant", "content": self.text},
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": self.input_tokens,
                "completion_tokens": self.output_tokens,
                "total_tokens": self.input_tokens.saturating_add(self.output_tokens),
            },
        }))
    }
}

fn assemble_sse(chunks: &[StreamChunk]) -> Option<GuardrailValue> {
    let mut builder = SseBuilder::default();
    for chunk in chunks {
        builder.absorb(chunk)?;
    }
    builder.finish()
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
            "index": 0_i32,
            "message": {"role": "assistant", "content": block.message},
            "finish_reason": "content_filter",
        }],
    }))
    .ok_or_else(block_failure)?;
    chat_chunk(&value)
        .map(|chunk| vec![chunk])
        .ok_or_else(block_failure)
}
