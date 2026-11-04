use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompatibleLineChunk {
    pub text: String,
    pub tool_use: Option<Value>,
    pub is_finished: bool,
    pub finish_reason: String,
    pub usage: Option<Value>,
    pub index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveError {
    Value(String),
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Chunk(CompatibleLineChunk),
    End,
    Error(String),
}

pub fn decode_legacy_compatible_line(line: &str, mode: Mode) -> Outcome {
    let stripped = line.strip_prefix("data:").unwrap_or(line).trim();
    let content = if stripped.is_empty() { "" } else { stripped };
    if mode == Mode::Async && content == "[DONE]" {
        return Outcome::End;
    }
    if content.is_empty() || content == "[DONE]" {
        return Outcome::Chunk(empty_chunk());
    }
    match serde_json::from_str::<Value>(content) {
        Ok(value) => Outcome::Chunk(project(value)),
        Err(_) => Outcome::Chunk(empty_chunk()),
    }
}

pub fn receive_failure(error: ReceiveError, mode: Mode) -> Outcome {
    match error {
        ReceiveError::Value(message) => {
            Outcome::Error(format!("Error receiving chunk from stream: {message}"))
        }
        ReceiveError::Other(message) if mode == Mode::Async => {
            Outcome::Error(format!("Error receiving chunk from stream: {message}"))
        }
        ReceiveError::Other(_) => Outcome::End,
    }
}

fn empty_chunk() -> CompatibleLineChunk {
    CompatibleLineChunk {
        text: String::new(),
        tool_use: None,
        is_finished: false,
        finish_reason: String::new(),
        usage: None,
        index: 0,
    }
}

fn project(value: Value) -> CompatibleLineChunk {
    let choices = value.get("choices").and_then(Value::as_array);
    let Some(first) = choices.and_then(|items| items.first()) else {
        let usage = value
            .get("usage")
            .filter(|item| !item.is_null())
            .map(|item| {
                let mut usage = item.clone();
                for field in ["prompt_tokens", "completion_tokens", "total_tokens"] {
                    if usage.get(field).is_none_or(Value::is_null)
                        || usage[field] == json!(0)
                        || usage[field] == json!(false)
                        || usage[field] == json!("")
                    {
                        usage[field] = json!(0);
                    }
                }
                usage
            });
        return CompatibleLineChunk {
            usage,
            ..empty_chunk()
        };
    };
    let delta = first.get("delta");
    let text = delta
        .and_then(|item| item.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tool_use = delta.and_then(|item| item.get("tool_calls")).and_then(Value::as_array).and_then(|calls| calls.first()).and_then(|call| {
        let function = call.get("function")?;
        if function.get("arguments")?.is_null() { return None; }
        Some(json!({"id": call.get("id"), "type":"function", "function":{"name":function.get("name"), "arguments":function.get("arguments")}, "index":call.get("index")}))
    });
    let reason = first.get("finish_reason").filter(|item| !item.is_null());
    let usage = value.get("usage").filter(|item| !item.is_null()).cloned();
    CompatibleLineChunk {
        text,
        tool_use,
        is_finished: reason.is_some(),
        finish_reason: reason
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        usage,
        index: 0,
    }
}

mod tests;
