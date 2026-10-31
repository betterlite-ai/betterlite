use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{MergeError, is_falsy, text_block};

// Assistant content: a string, a list of content blocks kept as-is, or any other JSON value LiteLLM stores unexamined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Value>),
    Other(Value),
}

impl Content {
    pub(super) fn is_falsy(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Blocks(items) => items.is_empty(),
            Self::Other(value) => is_falsy(value),
        }
    }

    // A string becomes one text block; a list is kept as-is, item shape unchecked, matching list[object].
    pub(super) fn into_blocks(self) -> Result<Vec<Value>, MergeError> {
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

// thinking_blocks: a non-empty list joins reasoning accumulation; any other JSON shape (a dict, a string, an empty list) is kept as-is but never merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThinkingBlocks {
    List(Vec<Value>),
    Other(Value),
}

impl ThinkingBlocks {
    pub(super) const fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) if !items.is_empty() => Some(items.as_slice()),
            Self::List(_) | Self::Other(_) => None,
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
    pub(super) fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) if !text.is_empty() => Some(text),
            Self::Text(_) | Self::Other(_) => None,
        }
    }
}
