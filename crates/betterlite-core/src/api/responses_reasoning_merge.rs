#![expect(
    clippy::disallowed_types,
    reason = "Reasoning thinking blocks and tool calls are opaque, provider-specific JSON that this module only reorders and concatenates, never interprets"
)]

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    #[error("assistant content must be a string or a list of text blocks")]
    InvalidContent,
}

// Public so merge_reasoning's trait bound is visible; lets one loop drive both record kinds.
pub trait Reasoning: Sized {
    fn is_pending(&self) -> bool;
    fn is_assistant(&self) -> bool;
    fn text(&self) -> Option<&str>;
    fn blocks(&self) -> Option<&[Value]>;
    fn set_text(&mut self, text: String);
    fn set_blocks(&mut self, blocks: Vec<Value>);
    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self;
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

fn field<'message>(message: &'message Value, name: &str) -> Option<&'message Value> {
    let fields = message.as_object()?;
    fields.get(name)
}

fn dict_text(message: &Value) -> Option<&str> {
    let value = field(message, "reasoning_content")?.as_str()?;
    (!value.is_empty()).then_some(value)
}

fn dict_blocks(message: &Value) -> Option<&[Value]> {
    let items = field(message, "thinking_blocks")?.as_array()?;
    (!items.is_empty()).then_some(items.as_slice())
}

fn content_is_none(message: &Value) -> bool {
    let content = field(message, "content");
    content.is_none_or(Value::is_null)
}

fn joined_text(texts: &[Option<&str>]) -> String {
    let present: Vec<&str> = texts.iter().filter_map(|value| *value).collect();
    present.join("\n")
}

fn concatenated_blocks(block_lists: &[Option<&[Value]>]) -> Vec<Value> {
    let mut merged = Vec::new();
    for blocks in block_lists.iter().flatten() {
        merged.extend(blocks.iter().cloned());
    }
    merged
}

impl Reasoning for Value {
    fn is_pending(&self) -> bool {
        field(self, "role").and_then(Self::as_str) == Some("assistant")
            && content_is_none(self)
            && field(self, "tool_calls").is_none_or(is_falsy)
            && (dict_text(self).is_some() || dict_blocks(self).is_some())
    }

    fn is_assistant(&self) -> bool {
        field(self, "role").and_then(Self::as_str) == Some("assistant")
    }

    fn text(&self) -> Option<&str> {
        dict_text(self)
    }

    fn blocks(&self) -> Option<&[Value]> {
        dict_blocks(self)
    }

    fn set_text(&mut self, text: String) {
        if let Some(fields) = self.as_object_mut() {
            fields.insert("reasoning_content".to_owned(), Self::String(text));
        }
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        if let Some(fields) = self.as_object_mut() {
            fields.insert("thinking_blocks".to_owned(), Self::Array(blocks));
        }
    }

    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self {
        let mut fields = Map::new();
        fields.insert("role".to_owned(), Self::String("assistant".to_owned()));
        fields.insert("content".to_owned(), Self::Null);
        if let Some(reasoning_content) = text {
            fields.insert(
                "reasoning_content".to_owned(),
                Self::String(reasoning_content),
            );
        }
        if let Some(thinking_blocks) = blocks {
            fields.insert("thinking_blocks".to_owned(), Self::Array(thinking_blocks));
        }
        Self::Object(fields)
    }
}

fn apply_pending<T>(target: &mut T, texts: &[Option<&str>], block_lists: &[Option<&[Value]>])
where
    T: Reasoning,
{
    let mut combined_texts: Vec<String> = texts
        .iter()
        .filter_map(|value| *value)
        .map(str::to_owned)
        .collect();
    if let Some(existing) = target.text() {
        combined_texts.push(existing.to_owned());
    }
    let mut combined_blocks = concatenated_blocks(block_lists);
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

fn standalone<T>(texts: &[Option<&str>], block_lists: &[Option<&[Value]>]) -> T
where
    T: Reasoning,
{
    let joined = joined_text(texts);
    let merged = concatenated_blocks(block_lists);
    T::blank(
        (!joined.is_empty()).then_some(joined),
        (!merged.is_empty()).then_some(merged),
    )
}

// Merges pending reasoning-only turns into the next assistant message, else rebuilds them standalone.
#[must_use]
pub fn merge_reasoning<T>(messages: &[T]) -> Vec<T>
where
    T: Reasoning + Clone,
{
    let mut merged = Vec::with_capacity(messages.len());
    let mut texts = Vec::new();
    let mut block_lists = Vec::new();
    for message in messages {
        if message.is_pending() {
            texts.push(message.text());
            block_lists.push(message.blocks());
            continue;
        }
        let mut carried = message.clone();
        if texts.is_empty() && block_lists.is_empty() {
            merged.push(carried);
            continue;
        }
        if carried.is_assistant() {
            apply_pending(&mut carried, &texts, &block_lists);
        } else {
            for (text, block) in texts.iter().zip(block_lists.iter()) {
                merged.push(standalone(&[*text], &[*block]));
            }
        }
        texts.clear();
        block_lists.clear();
        merged.push(carried);
    }
    for (text, block) in texts.iter().zip(block_lists.iter()) {
        merged.push(standalone(&[*text], &[*block]));
    }
    merged
}

fn text_block(text: &str) -> Value {
    let mut fields = Map::new();
    fields.insert("type".to_owned(), Value::String("text".to_owned()));
    fields.insert("text".to_owned(), Value::String(text.to_owned()));
    Value::Object(fields)
}

fn validate_text_block(item: &Value) -> Result<Value, MergeError> {
    let fields = item.as_object().ok_or(MergeError::InvalidContent)?;
    let is_text = fields.get("type").and_then(Value::as_str) == Some("text")
        && fields.get("text").and_then(Value::as_str).is_some();
    is_text
        .then(|| item.clone())
        .ok_or(MergeError::InvalidContent)
}

fn content_blocks(value: &Value) -> Result<Vec<Value>, MergeError> {
    match value {
        Value::String(text) => Ok(vec![text_block(text)]),
        Value::Array(items) => items.iter().map(validate_text_block).collect(),
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
    let Some(new) = new_messages.first().and_then(Value::as_object) else {
        return Ok(None);
    };
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
