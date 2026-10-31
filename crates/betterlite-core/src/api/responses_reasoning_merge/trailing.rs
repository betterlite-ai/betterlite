use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::content::Content;
use super::{MergeError, is_falsy};

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
