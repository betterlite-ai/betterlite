use serde::Deserialize;
use serde_json::{Map, Value};

use super::content::Content;
use super::{MergeError, Role, is_falsy};

// Any role outside the known chat roles is accepted and simply treated as not matching, never rejected.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum TrailingRole {
    Known(Role),
    Other(Value),
}

// Collapses "missing" and an explicit null into one falsy state, since mergeability treats them alike.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum ToolCalls {
    #[default]
    Absent,
    Present(Value),
}

impl ToolCalls {
    fn is_present_and_truthy(&self) -> bool {
        match self {
            Self::Present(value) => !is_falsy(value),
            Self::Absent => false,
        }
    }
}

// The trailing assistant message as merge_trailing reads it.
#[derive(Debug, Deserialize)]
struct TrailingRecord {
    #[serde(default)]
    role: Option<TrailingRole>,
    #[serde(default)]
    content: Option<Content>,
    #[serde(default)]
    tool_calls: ToolCalls,
}

// True once both records are assistant-authored and only the trailing one already carries tool calls (or hosted_search is set).
fn is_mergeable_pair(last: &TrailingRecord, new: &TrailingRecord, hosted_search: bool) -> bool {
    let is_assistant =
        |record: &TrailingRecord| matches!(record.role, Some(TrailingRole::Known(Role::Assistant)));
    is_assistant(last)
        && is_assistant(new)
        && (last.tool_calls.is_present_and_truthy() || hosted_search)
        && !new.tool_calls.is_present_and_truthy()
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
    let Some(Value::Object(last_map)) = messages.last() else {
        return Ok(None);
    };
    let last: TrailingRecord = serde_json::from_value(Value::Object(last_map.clone()))
        .map_err(|source| MergeError::InvalidRecord { source })?;
    let Some(new_value) = new_messages.first() else {
        return Ok(None);
    };
    // A map first, so a non-object new message raises instead of deserializing positionally.
    let new_map: Map<String, Value> = serde_json::from_value(new_value.clone())
        .map_err(|source| MergeError::InvalidRecord { source })?;
    let new: TrailingRecord = serde_json::from_value(Value::Object(new_map))
        .map_err(|source| MergeError::InvalidRecord { source })?;
    if !is_mergeable_pair(&last, &new, hosted_search) {
        return Ok(None);
    }
    let Some(new_content) = new.content else {
        return Ok(None);
    };
    let content = merged_content(&last, new_content)?;
    let mut merged: Map<String, Value> = last_map.clone();
    merged.insert("content".to_owned(), content);
    Ok(Some(Value::Object(merged)))
}
