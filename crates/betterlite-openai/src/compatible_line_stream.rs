use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Outcome {
    Chunk(CompatibleLineChunk),
    End,
    Error(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamInput {
    pub mode: ModeInput,
    pub events: Vec<StreamEvent>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeInput {
    Sync,
    Async,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StreamEvent {
    Line { line: String },
    ValueError { message: String },
    ReceiveError { message: String },
}

impl ModeInput {
    fn mode(self) -> Mode {
        match self {
            Self::Sync => Mode::Sync,
            Self::Async => Mode::Async,
        }
    }
}

pub fn run_stream(input: &StreamInput) -> Vec<Outcome> {
    let mode = input.mode.mode();
    let mut output = Vec::new();
    for event in &input.events {
        let outcome = match event {
            StreamEvent::Line { line } => decode_legacy_compatible_line(line, mode),
            StreamEvent::ValueError { message } => {
                receive_failure(ReceiveError::Value(message.clone()), mode)
            }
            StreamEvent::ReceiveError { message } => {
                receive_failure(ReceiveError::Other(message.clone()), mode)
            }
        };
        if outcome == Outcome::End || matches!(outcome, Outcome::Error(_)) {
            output.push(outcome);
            break;
        }
        output.push(outcome);
    }
    if !matches!(output.last(), Some(Outcome::End | Outcome::Error(_))) {
        output.push(Outcome::End);
    }
    output
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
        Err(error) => {
            tracing::debug!(error = %error, line = content, "Error parsing chunk; defaulting to empty chunk");
            Outcome::Chunk(empty_chunk())
        }
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
        ReceiveError::Other(message) => Outcome::Error(message),
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
            .map(|item| usage_projection(item, true));
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
    let reason = first
        .get("finish_reason")
        .filter(|item| !item.is_null() && item.as_str() != Some(""));
    let usage = value
        .get("usage")
        .filter(|item| !item.is_null())
        .map(|item| usage_projection(item, false));
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

fn usage_projection(value: &Value, default_falsey: bool) -> Value {
    let mut usage = serde_json::Map::new();
    for field in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        let token_count = value.get(field).cloned().unwrap_or(Value::Null);
        let is_falsey = token_count.is_null()
            || token_count == json!(0)
            || token_count == json!(false)
            || token_count == json!("");
        usage.insert(
            field.to_owned(),
            if default_falsey && is_falsey {
                json!(0)
            } else {
                token_count
            },
        );
    }
    Value::Object(usage)
}

mod tests;
