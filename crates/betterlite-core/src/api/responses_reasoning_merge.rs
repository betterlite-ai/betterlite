#![expect(
    clippy::disallowed_types,
    reason = "Reasoning thinking blocks and tool calls are opaque, provider-specific JSON that this module only reorders and concatenates, never interprets"
)]

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::rust::double_option;
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
    let mut fields = Map::new();
    fields.insert("type".to_owned(), Value::String("text".to_owned()));
    fields.insert("text".to_owned(), Value::String(text.to_owned()));
    Value::Object(fields)
}

// Assistant content: a string, a list of content blocks kept as-is, or any other JSON value LiteLLM stores unexamined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Value>),
    Other(Value),
}

impl Content {
    fn is_falsy(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Blocks(items) => items.is_empty(),
            Self::Other(value) => is_falsy(value),
        }
    }

    // A string becomes one text block; a list is kept as-is, item shape unchecked, matching list[object].
    fn into_blocks(self) -> Result<Vec<Value>, MergeError> {
        match self {
            Self::Text(text) => Ok(vec![text_block(&text)]),
            Self::Blocks(items) => Ok(items),
            Self::Other(_) => Err(MergeError::InvalidContent),
        }
    }
}

impl From<Content> for Value {
    fn from(content: Content) -> Self {
        match content {
            Content::Text(text) => Self::String(text),
            Content::Blocks(items) => Self::Array(items),
            Content::Other(value) => value,
        }
    }
}

// reasoning_content: any JSON is accepted, but only a non-empty string counts as reasoning text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReasoningText {
    Text(String),
    Other(Value),
}

impl ReasoningText {
    fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) if !text.is_empty() => Some(text),
            Self::Text(_) | Self::Other(_) => None,
        }
    }
}

// A chat message, dict or attribute shaped, with double_option keeping "missing" distinct from an explicit null.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRecord {
    pub role: Role,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub content: Option<Option<Content>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub reasoning_content: Option<Option<ReasoningText>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub thinking_blocks: Option<Option<Vec<Value>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub tool_calls: Option<Option<Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Reasoning for ChatRecord {
    fn is_pending(&self) -> bool {
        self.role == Role::Assistant
            && self.content.as_ref().and_then(Option::as_ref).is_none()
            && self
                .tool_calls
                .as_ref()
                .and_then(Option::as_ref)
                .is_none_or(is_falsy)
            && (self.text().is_some() || self.blocks().is_some())
    }

    fn is_assistant(&self) -> bool {
        self.role == Role::Assistant
    }

    fn text(&self) -> Option<&str> {
        self.reasoning_content
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(ReasoningText::as_text)
    }

    fn blocks(&self) -> Option<&[Value]> {
        self.thinking_blocks
            .as_ref()
            .and_then(Option::as_ref)
            .map(Vec::as_slice)
            .filter(|items| !items.is_empty())
    }

    fn set_text(&mut self, text: String) {
        self.reasoning_content = Some(Some(ReasoningText::Text(text)));
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        self.thinking_blocks = Some(Some(blocks));
    }

    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self {
        let mut record = Self {
            role: Role::Assistant,
            content: Some(None),
            reasoning_content: None,
            thinking_blocks: None,
            tool_calls: None,
            extra: Map::new(),
        };
        if let Some(found) = text {
            record.set_text(found);
        }
        if let Some(found) = blocks {
            record.set_blocks(found);
        }
        record
    }
}

// Tags whether a message came from a Python dict or an attribute object; LiteLLM dispatches per message, so one call can mix both kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Dict(ChatRecord),
    Attribute(ChatRecord),
}

impl Record {
    const fn inner(&self) -> &ChatRecord {
        let (Self::Dict(record) | Self::Attribute(record)) = self;
        record
    }

    const fn inner_mut(&mut self) -> &mut ChatRecord {
        let (Self::Dict(record) | Self::Attribute(record)) = self;
        record
    }
}

impl Reasoning for Record {
    fn is_pending(&self) -> bool {
        self.inner().is_pending()
    }

    fn is_assistant(&self) -> bool {
        self.inner().is_assistant()
    }

    fn text(&self) -> Option<&str> {
        self.inner().text()
    }

    fn blocks(&self) -> Option<&[Value]> {
        self.inner().blocks()
    }

    fn set_text(&mut self, text: String) {
        self.inner_mut().set_text(text);
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        self.inner_mut().set_blocks(blocks);
    }

    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self {
        Self::Dict(ChatRecord::blank(text, blocks))
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

// The trailing assistant message as merge_trailing reads and rebuilds it; role stays raw JSON since it is only ever compared against the "assistant" literal.
#[derive(Debug, Deserialize, Serialize)]
pub struct TrailingRecord {
    #[serde(default)]
    pub role: Value,
    #[serde(default)]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// True once both records are assistant-authored and only the trailing one already carries tool calls (or hosted_search is set).
fn is_mergeable_pair(last: &TrailingRecord, new: &TrailingRecord, hosted_search: bool) -> bool {
    let is_assistant = |record: &TrailingRecord| record.role.as_str() == Some("assistant");
    let has_tool_calls = |record: &TrailingRecord| {
        record
            .tool_calls
            .as_ref()
            .is_some_and(|value| !is_falsy(value))
    };
    is_assistant(last)
        && is_assistant(new)
        && (has_tool_calls(last) || hosted_search)
        && !has_tool_calls(new)
}

// Concatenates the trailing message's own content blocks ahead of the new message's, unless the trailing content is falsy.
fn merged_content(last: &TrailingRecord, new_content: Content) -> Result<Value, MergeError> {
    match &last.content {
        Some(previous) if !previous.is_falsy() => {
            let mut blocks = previous.clone().into_blocks()?;
            blocks.extend(new_content.into_blocks()?);
            Ok(Value::Array(blocks))
        }
        Some(Content::Text(_) | Content::Blocks(_) | Content::Other(_)) | None => {
            Ok(Value::from(new_content))
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
    let Some(last_value) = messages.last().filter(|value| value.is_object()) else {
        return Ok(None);
    };
    let last: TrailingRecord =
        serde_json::from_value(last_value.clone()).map_err(|_error| MergeError::InvalidRecord)?;
    let new_value = new_messages
        .first()
        .cloned()
        .ok_or(MergeError::InvalidRecord)?;
    let new: TrailingRecord =
        serde_json::from_value(new_value).map_err(|_error| MergeError::InvalidRecord)?;
    if !is_mergeable_pair(&last, &new, hosted_search) {
        return Ok(None);
    }
    let Some(new_content) = new.content else {
        return Ok(None);
    };
    let content = merged_content(&last, new_content)?;
    let Value::Object(mut merged) =
        serde_json::to_value(&last).map_err(|_error| MergeError::InvalidRecord)?
    else {
        return Err(MergeError::InvalidRecord);
    };
    merged.insert("content".to_owned(), content);
    Ok(Some(Value::Object(merged)))
}

mod tests;
