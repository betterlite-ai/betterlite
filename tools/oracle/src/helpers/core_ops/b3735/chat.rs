use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::model_armor::streams::assembly::Assembly;
use betterlite_core::guardrails::model_armor::streams::chunk::{Chunk, field, text_field};

use super::WireError;

const DEFAULT_ROLE: &str = "assistant";

#[derive(Default)]
struct Piece {
    content: String,
    role: Option<String>,
    finish_reason: Option<String>,
}

// LiteLLM's stream_chunk_builder, reduced to the fields a scan and a replay read.
pub(super) fn assemble(chunks: &[Chunk]) -> Option<Assembly> {
    let mut id = None;
    let mut model = None;
    let mut created = None;
    let mut choices: BTreeMap<i64, Piece> = BTreeMap::new();
    let mut seen = false;
    for chunk in chunks {
        let Some(payload) = chunk.payload() else {
            continue;
        };
        seen = true;
        take_text(payload, "id", &mut id);
        take_text(payload, "model", &mut model);
        if let Some(GuardrailValue::SignedInteger(value)) = field(payload, "created") {
            created = Some(*value);
        }
        let Some(GuardrailValue::Array(items)) = field(payload, "choices") else {
            continue;
        };
        for choice in items {
            absorb(choice, &mut choices);
        }
    }
    if !seen {
        return None;
    }
    let mut entries = Vec::with_capacity(choices.len());
    for (index, piece) in choices {
        entries.push(piece.into_choice(index));
    }
    Some(Assembly::Model(body(id, model, created, entries)))
}

fn take_text(payload: &GuardrailValue, name: &str, slot: &mut Option<String>) {
    if slot.is_none()
        && let Some(GuardrailValue::Text(text)) = field(payload, name)
    {
        *slot = Some(text.clone());
    }
}

fn absorb(choice: &GuardrailValue, choices: &mut BTreeMap<i64, Piece>) {
    let index = match field(choice, "index") {
        Some(GuardrailValue::SignedInteger(index)) => *index,
        _ => 0,
    };
    let piece = choices.entry(index).or_default();
    if let Some(delta) = field(choice, "delta")
        && let Some(GuardrailValue::Text(text)) = field(delta, "content")
    {
        piece.content.push_str(text);
    }
    if let Some(delta) = field(choice, "delta")
        && let Some(GuardrailValue::Text(role)) = field(delta, "role")
    {
        piece.role = Some(role.clone());
    }
    if let Some(GuardrailValue::Text(reason)) = field(choice, "finish_reason") {
        piece.finish_reason = Some(reason.clone());
    }
}

impl Piece {
    fn into_choice(self, index: i64) -> GuardrailValue {
        let role = match self.role {
            Some(role) => role,
            None => DEFAULT_ROLE.to_owned(),
        };
        let mut message = BTreeMap::new();
        message.insert("role".to_owned(), GuardrailValue::Text(role));
        message.insert("content".to_owned(), GuardrailValue::Text(self.content));
        let mut choice = BTreeMap::new();
        choice.insert("index".to_owned(), GuardrailValue::SignedInteger(index));
        choice.insert("message".to_owned(), GuardrailValue::Object(message));
        choice.insert("finish_reason".to_owned(), option_text(self.finish_reason));
        GuardrailValue::Object(choice)
    }
}

fn option_text(value: Option<String>) -> GuardrailValue {
    value.map_or(GuardrailValue::Null, GuardrailValue::Text)
}

fn body(
    id: Option<String>,
    model: Option<String>,
    created: Option<i64>,
    entries: Vec<GuardrailValue>,
) -> GuardrailValue {
    let mut body = BTreeMap::new();
    body.insert(
        "object".to_owned(),
        GuardrailValue::Text("chat.completion".to_owned()),
    );
    if let Some(id) = id {
        body.insert("id".to_owned(), GuardrailValue::Text(id));
    }
    if let Some(model) = model {
        body.insert("model".to_owned(), GuardrailValue::Text(model));
    }
    if let Some(created) = created {
        body.insert("created".to_owned(), GuardrailValue::SignedInteger(created));
    }
    body.insert("choices".to_owned(), GuardrailValue::Array(entries));
    GuardrailValue::Object(body)
}

// LiteLLM's MockResponseIterator: the sanitized response replayed as one stream chunk.
pub(super) fn emit(body: &GuardrailValue) -> Result<Vec<Chunk>, WireError> {
    let id = required_text(body, "id")?;
    let model = required_text(body, "model")?;
    let created = match field(body, "created") {
        Some(GuardrailValue::SignedInteger(value)) => *value,
        _ => return Err(WireError::Missing("created")),
    };
    let choices = emitted_choices(body)?;
    let mut chunk = BTreeMap::new();
    chunk.insert("id".to_owned(), GuardrailValue::Text(id.to_owned()));
    chunk.insert(
        "object".to_owned(),
        GuardrailValue::Text("chat.completion.chunk".to_owned()),
    );
    chunk.insert("created".to_owned(), GuardrailValue::SignedInteger(created));
    chunk.insert("model".to_owned(), GuardrailValue::Text(model.to_owned()));
    chunk.insert("choices".to_owned(), GuardrailValue::Array(choices));
    Ok(vec![Chunk::Json(GuardrailValue::Object(chunk))])
}

fn required_text<'value>(
    body: &'value GuardrailValue,
    name: &'static str,
) -> Result<&'value str, WireError> {
    match field(body, name) {
        Some(GuardrailValue::Text(text)) => Ok(text),
        _ => Err(WireError::Missing(name)),
    }
}

fn emitted_choices(body: &GuardrailValue) -> Result<Vec<GuardrailValue>, WireError> {
    let Some(GuardrailValue::Array(items)) = field(body, "choices") else {
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

fn emitted_choice(item: &GuardrailValue, position: i64) -> Result<GuardrailValue, WireError> {
    let Some(message) = field(item, "message") else {
        return Err(WireError::Missing("message"));
    };
    let content = match field(message, "content") {
        Some(GuardrailValue::Text(text)) => text.clone(),
        _ => String::new(),
    };
    let role = match text_field(field(message, "role")) {
        Some(role) => role.to_owned(),
        None => DEFAULT_ROLE.to_owned(),
    };
    let index = match field(item, "index") {
        Some(GuardrailValue::SignedInteger(index)) => *index,
        _ => position,
    };
    let finish_reason = plain(field(item, "finish_reason"));
    let mut delta = BTreeMap::new();
    delta.insert("role".to_owned(), GuardrailValue::Text(role));
    delta.insert("content".to_owned(), GuardrailValue::Text(content));
    let mut choice = BTreeMap::new();
    choice.insert("index".to_owned(), GuardrailValue::SignedInteger(index));
    choice.insert("delta".to_owned(), GuardrailValue::Object(delta));
    choice.insert("finish_reason".to_owned(), finish_reason);
    Ok(GuardrailValue::Object(choice))
}

fn plain(value: Option<&GuardrailValue>) -> GuardrailValue {
    value.cloned().unwrap_or(GuardrailValue::Null)
}
