use std::str::from_utf8;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_core::guardrails::model_armor::streams::assembly::Assembly;
use betterlite_core::guardrails::model_armor::streams::chunk::Chunk;
use betterlite_core::guardrails::model_armor::streams::errors::HttpError;
use betterlite_guardrails::anthropic_sse::sse_chunks_from_response::sse_chunks_from_response;
use serde_json::{Map, Value, json};

use super::WireError;

const DATA_PREFIX: &str = "data:";

#[derive(Default)]
struct Assembled {
    id: Option<String>,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    text: String,
    stop_reason: Option<String>,
    started: bool,
}

// LiteLLM's assemble_anthropic_sse_stream, reduced to the fields a scan and a replay read.
pub(super) fn assemble(chunks: &[Chunk]) -> Option<Assembly> {
    let mut state = Assembled::default();
    for chunk in chunks {
        let Chunk::Bytes(bytes) = chunk else {
            continue;
        };
        let Ok(frame) = from_utf8(bytes) else {
            continue;
        };
        let Some(event) = event_payload(frame) else {
            continue;
        };
        state.absorb(&event);
    }
    if !state.started {
        return None;
    }
    Some(Assembly::Model(state.into_body()?))
}

fn event_payload(frame: &str) -> Option<Value> {
    let data = frame
        .lines()
        .find_map(|line| line.strip_prefix(DATA_PREFIX))?;
    serde_json::from_str(data.trim()).ok()
}

impl Assembled {
    fn absorb(&mut self, event: &Value) {
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.started = true;
                let Some(message) = event.get("message") else {
                    return;
                };
                if let Some(id) = message.get("id").and_then(Value::as_str) {
                    self.id = Some(id.to_owned());
                }
                if let Some(model) = message.get("model").and_then(Value::as_str) {
                    self.model = Some(model.to_owned());
                }
                if let Some(usage) = message.get("usage").filter(|usage| !usage.is_null()) {
                    self.input_tokens = usage.get("input_tokens").and_then(integer);
                    self.output_tokens = usage.get("output_tokens").and_then(integer);
                }
            }
            Some("content_block_delta") => {
                if let Some(text) = event
                    .get("delta")
                    .and_then(|fields| fields.get("text"))
                    .and_then(Value::as_str)
                {
                    self.text.push_str(text);
                }
            }
            Some("message_delta") => {
                if let Some(reason) = event
                    .get("delta")
                    .and_then(|fields| fields.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop_reason = Some(reason.to_owned());
                }
                if let Some(tokens) = event
                    .get("usage")
                    .and_then(|fields| fields.get("output_tokens"))
                    .and_then(integer)
                {
                    self.output_tokens = Some(tokens);
                }
            }
            Some(_) | None => {}
        }
    }

    fn into_body(self) -> Option<GuardrailValue> {
        let reason = self.stop_reason.as_deref().map_or_else(
            || "stop".to_owned(),
            |reason| openai_reason(reason).to_owned(),
        );
        let mut body = Map::new();
        if let Some(id) = self.id {
            body.insert("id".to_owned(), Value::String(id));
        }
        if let Some(model) = self.model {
            body.insert("model".to_owned(), Value::String(model));
        }
        body.insert(
            "object".to_owned(),
            Value::String("chat.completion".to_owned()),
        );
        body.insert(
            "choices".to_owned(),
            json!([{
                "index": 0,
                "message": {"role": "assistant", "content": self.text},
                "finish_reason": reason,
            }]),
        );
        body.insert(
            "usage".to_owned(),
            json!({
                "prompt_tokens": self.input_tokens.unwrap_or(0),
                "completion_tokens": self.output_tokens.unwrap_or(0),
            }),
        );
        serde_json::from_value(Value::Object(body)).ok()
    }
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|raw| i64::try_from(raw).ok())),
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            None
        }
    }
}

fn openai_reason(reason: &str) -> &str {
    match reason {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

// LiteLLM's anthropic_sse_chunks_from_response: the sanitized response replayed as Anthropic frames.
pub(super) fn emit(body: &GuardrailValue) -> Result<Vec<Chunk>, WireError> {
    let plain = serde_json::to_value(body).map_err(WireError::Decode)?;
    let record: ResponseRecord = serde_json::from_value(plain).map_err(WireError::Decode)?;
    let frames = sse_chunks_from_response(&record)?;
    Ok(frames.into_iter().map(Chunk::Bytes).collect())
}

// LiteLLM's build_stream_error_items: one ErrorEvent carrying the status the frame would otherwise lose.
pub(super) fn error_item(error: &HttpError) -> Option<GuardrailValue> {
    let item = json!({
        "type": "error",
        "sequence_number": 0,
        "error": {
            "type": "guardrail_error",
            "code": error.status.to_string(),
            "message": detail_message(&error.detail),
            "param": null,
        },
    });
    serde_json::from_value(item).ok()
}

fn detail_message(detail: &GuardrailValue) -> String {
    let Ok(value) = serde_json::to_value(detail) else {
        return String::new();
    };
    let Value::Object(fields) = &value else {
        return text_or_encoded(&value);
    };
    if let Some(text) = fields.get("error").and_then(Value::as_str) {
        return text.to_owned();
    }
    if let Some(message) = fields
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return message.to_owned();
    }
    if let Some(message) = fields.get("message").and_then(Value::as_str) {
        return message.to_owned();
    }
    serde_json::to_string(&value).unwrap_or_default()
}

fn text_or_encoded(detail: &Value) -> String {
    match detail {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}
