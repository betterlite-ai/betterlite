use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::rust::double_option;

use super::content::Content;
use super::{MergeError, Role, is_empty_value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrailingRecord {
    #[serde(default, skip_serializing_if = "Role::is_missing")]
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
    pub tool_calls: Option<Option<Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrailingInput {
    Record(TrailingRecord),
    Other(Value),
}

impl TrailingInput {
    pub fn from_value(value: Value) -> Result<Self, MergeError> {
        match value {
            Value::Object(fields) => serde_json::from_value(Value::Object(fields))
                .map(Self::Record)
                .map_err(|source| MergeError::InvalidRecord { source }),
            Value::Null => Ok(Self::Other(Value::Null)),
            other @ (Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
                Ok(Self::Other(other))
            }
        }
    }
}

#[derive(Clone, Copy)]
enum ToolCalls<'value> {
    Absent,
    Present(&'value Value),
}

const fn tool_calls(record: &TrailingRecord) -> ToolCalls<'_> {
    match &record.tool_calls {
        Some(Some(value)) => ToolCalls::Present(value),
        Some(None) | None => ToolCalls::Absent,
    }
}

fn has_nonempty_tool_calls(calls: ToolCalls<'_>) -> bool {
    match calls {
        ToolCalls::Present(value) => !is_empty_value(value),
        ToolCalls::Absent => false,
    }
}

fn is_mergeable_pair(last: &TrailingRecord, new: &TrailingRecord, hosted_search: bool) -> bool {
    let last_is_assistant = matches!(last.role, Role::Known(super::KnownRole::Assistant));
    let new_is_assistant = matches!(new.role, Role::Known(super::KnownRole::Assistant));
    last_is_assistant
        && new_is_assistant
        && (has_nonempty_tool_calls(tool_calls(last)) || hosted_search)
        && !has_nonempty_tool_calls(tool_calls(new))
}

fn merged_content(last: &TrailingRecord, new_content: Content) -> Result<Content, MergeError> {
    match last.content.as_ref().and_then(Option::as_ref) {
        Some(previous) if !previous.is_empty_value() => {
            let mut blocks = previous.clone().into_blocks()?;
            blocks.extend(new_content.into_blocks()?);
            Ok(Content::Blocks(blocks))
        }
        Some(_) | None => Ok(new_content),
    }
}

pub fn merge_trailing(
    messages: &[TrailingInput],
    new_messages: &[TrailingInput],
    hosted_search: bool,
) -> Result<Option<TrailingRecord>, MergeError> {
    if messages.is_empty() || new_messages.len() != 1 {
        return Ok(None);
    }
    let Some(TrailingInput::Record(last)) = messages.last() else {
        return Ok(None);
    };
    let Some(TrailingInput::Record(new)) = new_messages.first() else {
        let value = new_messages
            .first()
            .map_or(Value::Null, |input| match input {
                TrailingInput::Other(other) => other.clone(),
                TrailingInput::Record(_) => Value::Null,
            });
        return match serde_json::from_value::<Map<String, Value>>(value) {
            Ok(_) => Ok(None),
            Err(source) => Err(MergeError::InvalidRecord { source }),
        };
    };
    if !is_mergeable_pair(last, new, hosted_search) {
        return Ok(None);
    }
    let Some(Some(new_content)) = new.content.clone() else {
        return Ok(None);
    };
    let mut merged = last.clone();
    merged.content = Some(Some(merged_content(last, new_content)?));
    Ok(Some(merged))
}

pub fn merge_trailing_values(
    messages: &[Value],
    new_messages: &[Value],
    hosted_search: bool,
) -> Result<Option<Value>, MergeError> {
    let typed_messages = messages
        .iter()
        .cloned()
        .map(TrailingInput::from_value)
        .collect::<Result<Vec<_>, _>>()?;
    let typed_new_messages = new_messages
        .iter()
        .cloned()
        .map(TrailingInput::from_value)
        .collect::<Result<Vec<_>, _>>()?;
    let record = merge_trailing(&typed_messages, &typed_new_messages, hosted_search)?;
    record
        .map(serde_json::to_value)
        .transpose()
        .map_err(|source| MergeError::InvalidRecord { source })
}
