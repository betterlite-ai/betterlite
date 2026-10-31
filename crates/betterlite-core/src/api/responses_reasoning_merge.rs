#![expect(
    clippy::disallowed_types,
    reason = "Reasoning thinking blocks and tool calls are opaque, provider-specific JSON that this module only reorders and concatenates, never interprets"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub mod content;
pub mod record;
pub mod trailing;

#[derive(Debug, Error)]
pub enum MergeError {
    #[error("assistant content must be a string or a list of content blocks")]
    InvalidContent,
    #[error("assistant message must be a JSON object: {source}")]
    InvalidRecord {
        #[source]
        source: serde_json::Error,
    },
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
pub trait Reasoning: Sized {
    fn is_pending(&self) -> bool;
    fn is_assistant(&self) -> bool;
    fn text(&self) -> Option<&str>;
    fn blocks(&self) -> Option<&[Value]>;
    fn set_text(&mut self, text: String);
    fn set_blocks(&mut self, blocks: Vec<Value>);

    // A blank, standalone assistant record built from just-merged reasoning text/blocks.
    #[must_use]
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

fn text_block(text: &str) -> Value {
    let mut fields = serde_json::Map::new();
    fields.insert("type".to_owned(), Value::String("text".to_owned()));
    fields.insert("text".to_owned(), Value::String(text.to_owned()));
    Value::Object(fields)
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

mod tests;
