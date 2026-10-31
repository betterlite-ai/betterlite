#![expect(
    clippy::disallowed_types,
    reason = "Reasoning thinking blocks and tool calls are opaque, provider-specific JSON that this module only reorders and concatenates, never interprets"
)]

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    #[error("assistant content must be a string or a list of content blocks")]
    InvalidContent,
    #[error("assistant message must be a JSON object")]
    InvalidRecord,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    Developer,
    User,
    #[default]
    Assistant,
    Tool,
    Function,
}

// Public so merge_reasoning's trait bound is visible; lets one loop drive both record kinds.
pub trait Reasoning: Sized + Default {
    fn is_pending(&self) -> bool;
    fn is_assistant(&self) -> bool;
    fn text(&self) -> Option<&str>;
    fn blocks(&self) -> Option<&[Value]>;
    fn set_text(&mut self, text: String);
    fn set_blocks(&mut self, blocks: Vec<Value>);

    // A blank, standalone assistant record built from just-merged reasoning text/blocks.
    #[must_use]
    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self {
        let mut record = Self::default();
        if let Some(found_text) = text {
            record.set_text(found_text);
        }
        if let Some(found_blocks) = blocks {
            record.set_blocks(found_blocks);
        }
        record
    }
}

fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
    }
}

// Dict-shaped chat message: the wire format LiteLLM feeds providers and the only shape the trailing merge accepts.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRecord {
    pub role: Role,
    #[serde(default)]
    pub content: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub reasoning_content: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub thinking_blocks: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub tool_calls: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Reasoning for ChatRecord {
    fn is_pending(&self) -> bool {
        self.role == Role::Assistant
            && self.content.is_null()
            && is_falsy(&self.tool_calls)
            && (self.text().is_some() || self.blocks().is_some())
    }

    fn is_assistant(&self) -> bool {
        self.role == Role::Assistant
    }

    fn text(&self) -> Option<&str> {
        self.reasoning_content
            .as_str()
            .filter(|value| !value.is_empty())
    }

    fn blocks(&self) -> Option<&[Value]> {
        self.thinking_blocks
            .as_array()
            .map(Vec::as_slice)
            .filter(|items| !items.is_empty())
    }

    fn set_text(&mut self, text: String) {
        self.reasoning_content = Value::String(text);
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        self.thinking_blocks = Value::Array(blocks);
    }
}

// Attribute-shaped chat record: LiteLLM reads these through getattr, so there is no passthrough bucket.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeRecord {
    pub role: Role,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,
}

impl Reasoning for AttributeRecord {
    fn is_pending(&self) -> bool {
        self.role == Role::Assistant
            && self.content.is_none()
            && self.tool_calls.as_ref().is_none_or(Vec::is_empty)
            && (self.text().is_some() || self.blocks().is_some())
    }

    fn is_assistant(&self) -> bool {
        self.role == Role::Assistant
    }

    fn text(&self) -> Option<&str> {
        self.reasoning_content
            .as_deref()
            .filter(|value| !value.is_empty())
    }

    fn blocks(&self) -> Option<&[Value]> {
        self.thinking_blocks
            .as_deref()
            .filter(|items| !items.is_empty())
    }

    fn set_text(&mut self, text: String) {
        self.reasoning_content = Some(text);
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        self.thinking_blocks = Some(blocks);
    }
}

// Prepends pending reasoning ahead of the target's own text/blocks, mutating it in place.
fn apply_pending<T>(target: &mut T, texts: &[Option<String>], block_lists: &[Option<Vec<Value>>])
where
    T: Reasoning,
{
    let mut combined_texts: Vec<String> = texts.iter().filter_map(Option::clone).collect();
    if let Some(existing) = target.text() {
        combined_texts.push(existing.to_owned());
    }
    let mut combined_blocks: Vec<Value> = block_lists
        .iter()
        .filter_map(Option::as_ref)
        .flat_map(|blocks| blocks.iter().cloned())
        .collect();
    if let Some(existing) = target.blocks() {
        combined_blocks.extend(existing.iter().cloned());
    }
    if !combined_texts.is_empty() {
        target.set_text(combined_texts.join("\n"));
    }
    if !combined_blocks.is_empty() {
        target.set_blocks(combined_blocks);
    }
}

// Merges pending reasoning-only turns into the next assistant message (mutated in place), else rebuilds them standalone.
#[must_use]
pub fn merge_reasoning<T>(messages: &mut [T]) -> Vec<T>
where
    T: Reasoning + Clone,
{
    let mut merged = Vec::with_capacity(messages.len());
    let mut texts: Vec<Option<String>> = Vec::new();
    let mut block_lists: Vec<Option<Vec<Value>>> = Vec::new();
    for message in messages.iter_mut() {
        if message.is_pending() {
            texts.push(message.text().map(str::to_owned));
            block_lists.push(message.blocks().map(<[Value]>::to_vec));
            continue;
        }
        if texts.is_empty() && block_lists.is_empty() {
            merged.push(message.clone());
            continue;
        }
        if message.is_assistant() {
            apply_pending(message, &texts, &block_lists);
        } else {
            for (text, blocks) in texts.iter().zip(block_lists.iter()) {
                merged.push(T::blank(text.clone(), blocks.clone()));
            }
        }
        texts.clear();
        block_lists.clear();
        merged.push(message.clone());
    }
    for (text, blocks) in texts.iter().zip(block_lists.iter()) {
        merged.push(T::blank(text.clone(), blocks.clone()));
    }
    merged
}

fn text_block(text: &str) -> Value {
    let mut fields = Map::new();
    fields.insert("type".to_owned(), Value::String("text".to_owned()));
    fields.insert("text".to_owned(), Value::String(text.to_owned()));
    Value::Object(fields)
}

// A string becomes one text block; a list is kept as-is, item shape unchecked, matching list[object].
fn content_blocks(value: &Value) -> Result<Vec<Value>, MergeError> {
    match value {
        Value::String(text) => Ok(vec![text_block(text)]),
        Value::Array(items) => Ok(items.clone()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
            Err(MergeError::InvalidContent)
        }
    }
}

// Merges a new assistant message into the trailing one only when the trailing message already carries tool calls or hosted_search is set and the new one doesn't, leaving its other fields untouched.
pub fn merge_trailing(
    messages: &[Value],
    new_messages: &[Value],
    hosted_search: bool,
) -> Result<Option<Value>, MergeError> {
    if messages.is_empty() || new_messages.len() != 1 {
        return Ok(None);
    }
    let Some(last) = messages.last().and_then(Value::as_object) else {
        return Ok(None);
    };
    let new = new_messages
        .first()
        .and_then(Value::as_object)
        .ok_or(MergeError::InvalidRecord)?;
    if last.get("role").and_then(Value::as_str) != Some("assistant")
        || new.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return Ok(None);
    }
    let last_has_tool_calls = last.get("tool_calls").is_some_and(|value| !is_falsy(value));
    let new_has_tool_calls = new.get("tool_calls").is_some_and(|value| !is_falsy(value));
    if !(last_has_tool_calls || hosted_search) || new_has_tool_calls {
        return Ok(None);
    }
    let Some(new_content) = new.get("content").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let content = match last.get("content") {
        Some(previous) if !is_falsy(previous) => {
            let mut blocks = content_blocks(previous)?;
            blocks.extend(content_blocks(new_content)?);
            Value::Array(blocks)
        }
        _ => new_content.clone(),
    };
    let mut result = last.clone();
    result.insert("content".to_owned(), content);
    Ok(Some(Value::Object(result)))
}

mod tests;
