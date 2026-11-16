use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::model_armor::streams::assembly::Assembly;
use betterlite_core::guardrails::model_armor::streams::chunk::Chunk;
use serde_json::{Map, Value, json};

use super::WireError;

const DEFAULT_ROLE: &str = "assistant";

#[derive(Default)]
struct Piece {
    content: String,
    role: Option<String>,
    finish_reason: Option<String>,
}

#[derive(Default)]
struct State {
    id: Option<String>,
    model: Option<String>,
    created: Option<i64>,
    choices: BTreeMap<i64, Piece>,
    seen: bool,
}

// LiteLLM's stream_chunk_builder, reduced to the fields a scan and a replay read.
pub(super) fn assemble(chunks: &[Chunk]) -> Option<Assembly> {
    let mut state = State::default();
    for chunk in chunks {
        state.absorb(chunk);
    }
    state.into_body()
}

impl State {
    fn absorb(&mut self, chunk: &Chunk) {
        let Some(payload) = chunk.payload() else {
            return;
        };
        self.seen = true;
        let Ok(value) = serde_json::to_value(payload) else {
            return;
        };
        if self.id.is_none() {
            self.id = value.get("id").and_then(Value::as_str).map(str::to_owned);
        }
        if self.model.is_none() {
            self.model = value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if let Some(number) = value.get("created").and_then(Value::as_i64) {
            self.created = Some(number);
        }
        let Some(items) = value.get("choices").and_then(Value::as_array) else {
            return;
        };
        for choice in items {
            absorb_choice(choice, &mut self.choices);
        }
    }

    fn into_body(self) -> Option<Assembly> {
        if !self.seen {
            return None;
        }
        let entries = self
            .choices
            .into_iter()
            .map(|(index, piece)| piece.into_choice(index))
            .collect();
        let body = completion_body(self.id, self.model, self.created, entries);
        serde_json::from_value(body).ok().map(Assembly::Model)
    }
}

fn absorb_choice(choice: &Value, choices: &mut BTreeMap<i64, Piece>) {
    let index = choice.get("index").and_then(Value::as_i64).unwrap_or(0);
    let piece = choices.entry(index).or_default();
    let delta = choice.get("delta");
    if let Some(content) = delta
        .and_then(|fields| fields.get("content"))
        .and_then(Value::as_str)
    {
        piece.content.push_str(content);
    }
    if let Some(role) = delta
        .and_then(|fields| fields.get("role"))
        .and_then(Value::as_str)
    {
        piece.role = Some(role.to_owned());
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        piece.finish_reason = Some(reason.to_owned());
    }
}

impl Piece {
    fn into_choice(self, index: i64) -> Value {
        json!({
            "index": index,
            "message": {
                "role": self.role.unwrap_or_else(|| DEFAULT_ROLE.to_owned()),
                "content": self.content,
            },
            "finish_reason": self.finish_reason.map_or(Value::Null, Value::String),
        })
    }
}

fn completion_body(
    id: Option<String>,
    model: Option<String>,
    created: Option<i64>,
    entries: Vec<Value>,
) -> Value {
    let mut body = Map::new();
    body.insert(
        "object".to_owned(),
        Value::String("chat.completion".to_owned()),
    );
    if let Some(id) = id {
        body.insert("id".to_owned(), Value::String(id));
    }
    if let Some(model) = model {
        body.insert("model".to_owned(), Value::String(model));
    }
    if let Some(created) = created {
        body.insert("created".to_owned(), Value::from(created));
    }
    body.insert("choices".to_owned(), Value::Array(entries));
    Value::Object(body)
}

// LiteLLM's MockResponseIterator: the sanitized response replayed as one stream chunk.
pub(super) fn emit(body: &GuardrailValue) -> Result<Vec<Chunk>, WireError> {
    let plain = serde_json::to_value(body).map_err(WireError::Decode)?;
    let id = required(&plain, "id")?;
    let model = required(&plain, "model")?;
    let created = plain
        .get("created")
        .and_then(Value::as_i64)
        .ok_or(WireError::Missing("created"))?;
    let chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": emitted_choices(&plain)?,
    });
    Ok(vec![Chunk::Json(
        serde_json::from_value(chunk).map_err(WireError::Decode)?,
    )])
}

fn required<'value>(body: &'value Value, name: &'static str) -> Result<&'value str, WireError> {
    body.get(name)
        .and_then(Value::as_str)
        .ok_or(WireError::Missing(name))
}

fn emitted_choices(body: &Value) -> Result<Vec<Value>, WireError> {
    let Some(items) = body.get("choices").and_then(Value::as_array) else {
        return Err(WireError::Missing("choices"));
    };
    let mut emitted = Vec::with_capacity(items.len());
    let mut position = 0_i64;
    for item in items {
        emitted.push(emitted_choice(item, position)?);
        position = position.saturating_add(1);
    }
    Ok(emitted)
}

fn emitted_choice(item: &Value, position: i64) -> Result<Value, WireError> {
    let Some(message) = item.get("message") else {
        return Err(WireError::Missing("message"));
    };
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_ROLE);
    let index = item
        .get("index")
        .and_then(Value::as_i64)
        .unwrap_or(position);
    Ok(json!({
        "index": index,
        "delta": {"role": role, "content": content},
        "finish_reason": item.get("finish_reason").cloned().unwrap_or(Value::Null),
    }))
}
