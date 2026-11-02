use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{MergeError, is_empty_value, text_block};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Value>),
    Other(Value),
}

impl Content {
    pub(super) fn is_empty_value(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Blocks(items) => items.is_empty(),
            Self::Other(value) => is_empty_value(value),
        }
    }

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
